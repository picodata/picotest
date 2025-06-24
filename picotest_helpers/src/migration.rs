use std::collections::HashMap;
use std::fs::{read_dir, read_to_string};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{bail, Context, Error};
use pike::cluster::MigrationContextVar;

#[derive(Debug, Clone)]
pub struct Migrations {
    sequence: Vec<Migration>,
}

impl Deref for Migrations {
    type Target = [Migration];
    fn deref(&self) -> &Self::Target {
        &self.sequence
    }
}

impl Migrations {
    pub fn from_unsorted(mut migrations: Vec<Migration>) -> Self {
        migrations.sort_by(|a, b| a.version.cmp(&b.version));
        Self {
            sequence: migrations,
        }
    }
}

pub type MigrationVersion = u32;

#[derive(Debug, Clone)]
pub struct Migration {
    version: MigrationVersion,
    name: String,
    statements: Vec<MigrationStatement>,
    up_range: (usize, usize),
    down_range: (usize, usize),
}

impl Migration {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn statements(&self) -> &[MigrationStatement] {
        &self.statements
    }

    pub fn up_statements(&self) -> &[MigrationStatement] {
        &self.statements[self.up_range.0..self.up_range.1]
    }

    pub fn down_statements(&self) -> &[MigrationStatement] {
        &self.statements[self.down_range.0..self.down_range.1]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStatement {
    original_text: String,
    modified_text: Option<String>,
}

impl MigrationStatement {
    pub fn new(text: impl ToString) -> Self {
        Self {
            original_text: text.to_string(),
            modified_text: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.original_text
    }

    pub fn is_line_comment(&self) -> bool {
        self.original_text.starts_with("--")
    }

    pub fn is_pico_up(&self) -> bool {
        self.original_text == "-- pico.UP"
    }

    pub fn is_pico_down(&self) -> bool {
        self.original_text == "-- pico.DOWN"
    }

    pub fn extract_tier_variables(&self) -> Vec<String> {
        // returns true, if character can not belong to identifier
        fn is_not_identifier_char(c: char) -> bool {
            !c.is_alphanumeric() && c != '_'
        }
        // extracts prefix, a longest identifier
        fn collect_variable_identifier(text: &str) -> &str {
            text.split_once(is_not_identifier_char)
                .map(|(before, _after)| before)
                .unwrap_or(text)
        }

        let pattern = "in tier @_plugin_config.";
        let get_text_after_pattern = |match_idx: usize| -> &str {
            let start_idx = match_idx + pattern.len();
            &self.original_text[start_idx..]
        };

        // match pattern, map occurrences into original-string-after-match, extract var names
        self.original_text
            .to_lowercase()
            .match_indices(pattern)
            .map(|(idx, _match)| get_text_after_pattern(idx))
            .map(collect_variable_identifier)
            .map(String::from)
            .collect::<Vec<_>>()
    }
}

impl From<String> for MigrationStatement {
    fn from(value: String) -> Self {
        MigrationStatement::new(value)
    }
}

/// Builds migration context variables for plugin by its name
pub trait MigrationContextProvider {
    fn get_migration_context(&self, plugin_name: &str) -> Vec<MigrationContextVar>;
}

impl MigrationContextProvider for Vec<MigrationContextVar> {
    fn get_migration_context(&self, _plugin_name: &str) -> Vec<MigrationContextVar> {
        self.clone()
    }
}

impl MigrationContextProvider for HashMap<String, Vec<MigrationContextVar>> {
    fn get_migration_context(&self, plugin_name: &str) -> Vec<MigrationContextVar> {
        self.get(plugin_name).cloned().unwrap_or_default()
    }
}

pub fn parse_migration_file_name<P>(file_name: P) -> Result<(MigrationVersion, String), Error>
where
    P: AsRef<Path>,
{
    let Some(file_name) = file_name.as_ref().file_name() else {
        bail!("migration file does not have file name")
    };
    let Some(file_name) = file_name.to_str() else {
        bail!("migration file have non-utf8 name")
    };
    let Some((name, ext)) = file_name.rsplit_once('.') else {
        bail!("migration file does not have an extension")
    };
    if ext.to_lowercase() != "sql" {
        bail!("migration file does not have sql extension")
    }
    let Some((version, migration_name)) = name.split_once('_') else {
        bail!("migration file has invalid name")
    };
    let Ok(version) = MigrationVersion::from_str(version) else {
        bail!("failed to parse migration version: {version}")
    };
    Ok((version, migration_name.to_string()))
}

pub fn parse_migration_text<S>(sql_text: S) -> Result<Vec<MigrationStatement>, Error>
where
    S: AsRef<str>,
{
    let sql_text = sql_text.as_ref();
    let mut output = Vec::with_capacity(sql_text.matches('\n').count());
    let mut acc = None;

    for line in sql_text.lines() {
        // skip empty lines
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // single line comment
        if line.starts_with("--") {
            // ignore if currently building a statement
            if acc.is_none() {
                output.push(MigrationStatement::new(line));
                continue;
            }
        }
        // append and insert statement text
        if let Some(acc_string) = acc.take() {
            acc = Some(acc_string + line)
        } else {
            acc = Some(String::from(line));
        }
        // statement was not finished, continue building
        if !line.ends_with(';') {
            continue;
        }
        let acc_string = acc.take().unwrap();
        output.push(MigrationStatement::new(acc_string));
    }
    Ok(output)
}

/// Extract indexes [a,b), where starts and ends migrations by type.
#[allow(clippy::type_complexity)]
fn extract_up_down_ranges(
    statements: &[MigrationStatement],
) -> Result<((usize, usize), (usize, usize)), Error> {
    let mut up_range_start = 0;
    let mut down_range_start = 0;
    let end_range = statements.len();
    for (idx, statement) in statements.iter().enumerate() {
        if statement.is_line_comment() {
            if statement.text().starts_with("-- pico.UP") {
                up_range_start = idx
            }
            if statement.text().starts_with("-- pico.DOWN") {
                down_range_start = idx
            }
        }
    }
    Ok((
        (up_range_start, down_range_start),
        (down_range_start, end_range),
    ))
}

fn parse_migration_file<P>(path: P) -> Result<Migration, Error>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();
    let file_content = read_to_string(path)?;
    let (version, name) = parse_migration_file_name(path)?;
    let statements = parse_migration_text(&file_content)?;
    let (up_range, down_range) = extract_up_down_ranges(&statements)?;
    Ok(Migration {
        version,
        name,
        statements,
        up_range,
        down_range,
    })
}

pub fn parse_migrations<P>(migrations_dir: P) -> Result<Migrations, Error>
where
    P: AsRef<Path>,
{
    let path = migrations_dir.as_ref();
    let dir = std::fs::read_dir(path).context("migration directory can not be read")?;
    let entries = dir.map(Result::unwrap).collect::<Vec<_>>();
    let mut migrations = Vec::with_capacity(entries.len());
    for entry in entries {
        migrations.push(parse_migration_file(entry.path())?);
    }
    Ok(Migrations::from_unsorted(migrations))
}

/// Tries to locate all directories with plugin migrations in given profile build
pub fn find_migrations_directories<P>(target_dir: P) -> Result<Vec<(String, PathBuf)>, Error>
where
    P: AsRef<Path>,
{
    fn allowlisted_dir_name(dir: &std::fs::DirEntry) -> bool {
        let blacklist = ["build", "deps", "examples", "incremental", ".fingerprint"];
        !blacklist.contains(&dir.file_name().to_string_lossy().as_ref())
            && dir.file_type().is_ok_and(|t| t.is_dir())
    }

    let mut output = Vec::new();
    let entries = read_dir(target_dir.as_ref())
        .context("reading plugin target directory for migrations search")?;
    for plugin_entry in entries.filter_map(Result::ok).filter(allowlisted_dir_name) {
        let plugin_name = plugin_entry.file_name().to_string_lossy().into_owned();
        let plugin_shipping_path = plugin_entry.path();
        let plugin_dir = read_dir(&plugin_shipping_path).with_context(|| {
            format!(
                "searching plugin directory {} for migrations",
                plugin_shipping_path.to_string_lossy()
            )
        })?;
        let mut versions = plugin_dir.filter_map(Result::ok).collect::<Vec<_>>();
        versions.sort_by_cached_key(|dir| dir.file_name());
        let Some(latest_version) = versions.last() else {
            continue;
        };
        let migrations_path = latest_version.path().join("migrations");
        if migrations_path.exists() {
            output.push((plugin_name, migrations_path));
        }
    }
    Ok(output)
}

pub fn make_ddl_tier_overrides(
    migrations: &Migrations,
    target_tier: &str,
) -> Vec<MigrationContextVar> {
    let mut output = Vec::new();
    for migration in migrations.iter() {
        for statement in migration.statements() {
            let ctx_var_names = statement.extract_tier_variables();
            for ctx_var_name in ctx_var_names {
                output.push(MigrationContextVar {
                    name: ctx_var_name,
                    value: target_tier.to_string(),
                });
            }
        }
    }
    output
}

#[cfg(test)]
mod test {
    use std::ffi::OsStr;

    use rstest::rstest;

    use crate::migration::make_ddl_tier_overrides;

    use super::{extract_up_down_ranges, parse_migration_file_name, parse_migration_text};
    use super::{Migration, MigrationStatement, Migrations};

    #[rstest]
    #[case::short_path("0001_first_migration.sql", 1, "first_migration")]
    #[case::full_path("/something/0002_second_migration.SQL", 2, "second_migration")]
    fn migration_file_name_parse_ok(
        #[case] file_name: &str,
        #[case] version: u32,
        #[case] m_name: &str,
    ) {
        let (v, name) = parse_migration_file_name(file_name).expect("should parse first migration");
        assert_eq!(v, version, "migration version does not match");
        assert_eq!(name, m_name, "migration name does not match");
    }

    #[rstest]
    #[case::no_file_name(OsStr::new(".."), "migration file does not have file name")]
    #[case::no_extension(OsStr::new("migration"), "migration file does not have an extension")]
    #[case::not_an_sql(OsStr::new("m.EXE"), "migration file does not have sql extension")]
    #[case::unpartable(OsStr::new("migration.sql"), "migration file has invalid name")]
    #[case::non_int_ver(OsStr::new("ver_migr.sql"), "failed to parse migration version: ver")]
    fn migration_file_name_parse_invalid(#[case] file_name: &OsStr, #[case] err_text: &str) {
        let error = parse_migration_file_name(file_name).expect_err("should fail");
        assert_eq!(error.to_string(), err_text);
    }

    #[rstest]
    fn migration_file_parse_single_line() {
        let text = r#"
        -- pico.UP
        CREATE TABLE t (id INTEGER NOT NULL, PRIMARY KEY (id)) USING memtx DISTRIBUTED BY (id) IN TIER @_plugin_config.custom_tier;
        -- pico.DOWN
        DROP TABLE t;
        "#;
        let parsed = parse_migration_text(text).unwrap();
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].text(), "-- pico.UP");
        assert!(parsed[0].is_line_comment());
        assert!(parsed[1].text().starts_with("CREATE TABLE t"));
        assert!(parsed[1].text().ends_with("custom_tier;"));
        assert_eq!(parsed[2].text(), "-- pico.DOWN");
        assert!(parsed[2].is_line_comment());
        assert!(parsed[3].text().starts_with("DROP TABLE t"));
    }

    #[rstest]
    fn migration_file_parse_multiline() {
        let text = r#"
        -- pico.UP
        CREATE TABLE t (
            id INTEGER NOT NULL,
            PRIMARY KEY (id)
        ) 
        USING memtx DISTRIBUTED by (id)
        in tier @_plugin_config.picotest_tier;
        CREATE TABLE a (
            id INTEGER
        )
        in TieR @_plugin_config.a_tier;

        -- pico.DOWN
        DROP TABLE t;

        DROP TABLE a;
        "#;
        let parsed = parse_migration_text(text).unwrap();
        assert_eq!(parsed.len(), 6);
        assert_eq!(parsed[0].text(), "-- pico.UP");
        let line_1 = parsed[1].text();
        assert!(!line_1.contains('\n'));
        assert!(line_1.ends_with("in tier @_plugin_config.picotest_tier;"));
        let line_2 = parsed[2].text();
        assert!(!line_2.contains('\n'));
        assert!(line_2.ends_with("in TieR @_plugin_config.a_tier;"));
        assert_eq!(parsed[3].text(), "-- pico.DOWN");
        assert_eq!(parsed[4].text(), "DROP TABLE t;");
        assert_eq!(parsed[5].text(), "DROP TABLE a;");
    }

    #[rstest]
    fn migration_extract_tier_variables() {
        let sql = "CREATE TABLE t() in Tier @_plugin_config.picotest_tier\n;";
        let statement = parse_migration_text(sql)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(
            statement.extract_tier_variables(),
            vec![String::from("picotest_tier")]
        )
    }

    fn into_statements(s: &[&str]) -> Vec<MigrationStatement> {
        s.iter().map(MigrationStatement::new).collect::<Vec<_>>()
    }

    #[rstest]
    fn migration_extract_up_down_range() {
        let statements = into_statements(&[
            "-- pico.UP",
            "CREATE TABLE t IN almost_pure_sql_tier;",
            "CREATE TABLE u in somethingsomething;",
            "-- pico.DOWN",
            "DROP TABLE t;",
            "DROP TABLE d;",
        ]);
        let (up, down) = extract_up_down_ranges(&statements).unwrap();
        assert_eq!(up, (0, 3));
        assert_eq!(down, (3, 6));
    }

    #[rstest]
    fn migration_sort_by_versions() {
        let migration = |ver, name: &str| Migration {
            version: ver,
            name: name.to_string(),
            statements: vec![],
            up_range: (0, 0),
            down_range: (0, 0),
        };
        let migrations = Migrations::from_unsorted(vec![
            migration(2, "second"),
            migration(22, "22"),
            migration(1, "first"),
            migration(0, "why_not"),
        ]);
        assert_eq!(migrations[0].name(), "why_not");
        assert_eq!(migrations[1].name(), "first");
        assert_eq!(migrations[2].name(), "second");
        assert_eq!(migrations[3].name(), "22");
    }

    #[rstest]
    fn migration_simple_ddl_tier_override() {
        let migrations = Migrations::from_unsorted(vec![Migration {
            version: 1,
            name: String::from("first"),
            statements: into_statements(&[
                "-- pico.UP",
                "CREATE TABLE table IN TIER @_plugin_config.storage;",
                "-- pico.DOWN",
                "CREATE TABLE table IN TIER @_plugin_config.router;",
            ]),
            up_range: (0, 0),
            down_range: (0, 0),
        }]);
        let ctx_vars = make_ddl_tier_overrides(&migrations, "default");
        assert_eq!(ctx_vars.len(), 2);
        assert_eq!(ctx_vars[0].name, "storage");
        assert_eq!(ctx_vars[0].value, "default");
        assert_eq!(ctx_vars[1].name, "router");
        assert_eq!(ctx_vars[1].value, "default");
    }
}
