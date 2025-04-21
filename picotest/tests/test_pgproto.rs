mod helpers;

use ctor::ctor;
use helpers::plugin;
use picotest::*;
use picotest_helpers::{PG_USER, PG_USER_PASSWORD};
use postgres::{Client, NoTls};

#[derive(Debug, PartialEq, Eq)]
struct User {
    id: i64,
    name: String,
    last_name: String,
}

#[ctor]
fn init_plugin() {
    plugin();
}

#[picotest(path = "../tmp/test_plugin")]
fn test_get_instances() {
    assert_eq!(cluster.instances().len(), 4);
    assert_eq!(cluster.main().pg_port(), &5433)
}

#[picotest(path = "../tmp/test_plugin")]
fn test_pg_connection() {
    let conn_string = format!(
        "host=localhost port={} user={} password={}",
        cluster.main().pg_port(),
        PG_USER,
        PG_USER_PASSWORD
    );
    let mut client = Client::connect(conn_string.as_str(), NoTls).unwrap();
    client
        .execute(
            "
            CREATE TABLE IF NOT EXISTS users (
            Id INT PRIMARY KEY,
            Name VARCHAR(50) NOT NULL,
            LastName VARCHAR(50) NOT NULL
        )",
            &[],
        )
        .unwrap();

    let user = User {
        id: 1,
        name: "Picotest".into(),
        last_name: "Picotest".into(),
    };

    client
        .execute(
            &format!(
                "INSERT INTO users (Id, Name, LastName) VALUES ({}, '{}', '{}')",
                user.id, user.name, user.last_name
            ),
            &[],
        )
        .unwrap();

    let users = client
        .query("SELECT Id, Name, LastName FROM users", &[])
        .unwrap()
        .iter()
        .map(|row| User {
            id: row.get("id"),
            name: row.get("Name"),
            last_name: row.get("LastName"),
        })
        .collect::<Vec<_>>();

    assert_eq!(users.len(), 1);

    let pg_user = users.first().unwrap();
    assert_eq!(&user, pg_user);
}
