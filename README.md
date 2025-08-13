# Test framework for Picodata plugin

[![CI](https://github.com/picodata/picotest/actions/workflows/build-and-test.yml/badge.svg)](https://github.com/picodata/picotest/actions/workflows/build-and-test.yml)

## Описание

**Picotest** - это фреймворк для тестирования плагинов, созданных в окружении [`pike`](https://github.com/picodata/pike).

Для использования **Picotest** требуется выполнить следующие действия:

- Установить [pike](https://crates.io/crates/picodata-pike):

```bash
cargo install picodata-pike
```

- Добавить зависимости в `Cargo.toml` плагина:

```bash
cargo add --dev picotest
cargo add --dev rstest
```

## Совместимость с Picodata

| Picotest | Picodata |
|-----------------|-----------------|
| >= 1.4.0        | 25.2.1          |
| >= 1.4.0        | 25.1.2          |
| < 1.4.0         | 25.1.1          |

## Интеграционное тестирование

Макрос `#[picotest]` используется для написания интеграционных тестов и может применяться как к функциям, так и к модулям.


### Использование `#[picotest]` 
При использовании макроса на модуле `picotest` автоматически пометит все функции модуля, названия которых начинаются с `test_`, как `rstest`-функции.

```rust
use picotest::*;

#[picotest]
mod test_mod {

    fn test_foo() {
        assert_eq!("foo", "foo");
    }

    fn test_bar() {
        assert_eq!("bar", "bar");
    }
}
```

### Совместимость с `rstest`

Макрос `#[picotest]` является оберткой над [`rstest`](https://github.com/la10736/rstest), поэтому поддерживает использование: 
[`fixture`](https://docs.rs/rstest/latest/rstest/attr.fixture.html).
[`once`](https://docs.rs/rstest/latest/rstest/attr.fixture.html#once-fixture)
[`case`](https://docs.rs/rstest/latest/rstest/attr.rstest.html#test-parametrized-cases)

```rust
use picotest::picotest;

#[picotest]
mod test_mod {
    #[fixture]
    fn foo() -> String {
        "foo".to_string()
    }

    #[fixture]
    #[once]
    fn bar() -> String {
        "bar".to_string()
    }

    fn test_foo(foo: String) {
        assert_eq!(foo, "foo".to_string());
    }

    fn test_bar(bar: String) {
        assert_eq!(bar, "bar".to_string());
    }

    fn test_foo_bar(foo: String, bar: String) {
        assert_ne!(foo, bar);
    }

    #[case(0, 0)]
    #[case(1, 1)]
    #[case(2, 1)]
    #[case(3, 2)]
    #[case(4, 3)]
    fn test_fibonacci(#[case] input: u32, #[case] expected: u32) {
        assert_eq!(expected, fibonacci(input))
    }
```

### Атрибуты макроса `#[picotest]`

| Attribute | Description | Default |
|-----------|-------------|---------|
| `path`    | Путь до директории, содержащей файл топологии плагина ([topology.toml](https://github.com/picodata/pike?tab=readme-ov-file#topologytoml)) | Определяется автоматически |
| `timeout` | Таймаут перед запуском первого теста (seconds) | 5 |

### Применение конфигурации плагина к запущенному кластеру Picodata

Picotest позволяет менять конфигурацию сервисов плагина во время исполнения интеграционного теста.

Изменение конфигурации происходит с помощью метода [Cluster::apply_config](https://docs.rs/picotest/latest/picotest/struct.Cluster.html#method.apply_config), 
который доступен при использовании макроса #[picotest].

Формат конфигурации должен соответствовать маппингу YAML, заданному в [plugin_config.yaml](https://github.com/picodata/pike?tab=readme-ov-file#config-apply).

**Пример:**

Допустим конфигурация плагина с одним сервисом "router" задана следующим образом:

```yaml
router:
    rpc_endpoint: "/hello"
    max_rpc_message_size_bytes: 1024
    max_rpc_message_queue_size: 2048
```

Тогда интеграционный тест, изменяющий данную конфигурацию, может выглядеть следующим образом:

```rust
#[picotest]
fn test_apply_plugin() {
    // 1. Assemble YAML string.

    let plugin_config_yaml = r#"
        router:
            rpc_endpoint: "/test"
            max_rpc_message_size_bytes: 128
            max_rpc_message_queue_size: 32
        "#;

    let plugin_config: PluginConfigMap = 
        serde_yaml::from_str(plugin_config_yaml).unwrap();

    // 2. Apply config to the running cluster instance.

    cluster // implicitly created variable by picotest magic
        .apply_config(plugin_config)
        .expect("Failed to apply config");

    // Callback Serivce::on_config_change should've been already
    // called at this point.
}
```

Подробнее в описании метода [Cluster::apply_config](https://docs.rs/picotest/latest/picotest/struct.Cluster.html#method.apply_config).

# Управление кластером в Picotest

## Жизненный цикл и изоляция кластера

Picotest обеспечивает полную изоляцию тестовых окружений за счет автоматического управления жизненным циклом кластера:

### Архитектура тестирования

```bash
my_pike_plugin/
├── topology.toml    # Файл топологии плагина
├── src/
│   └── lib.rs       # Основной код
└── tests/
    ├── common/      # Вспомогательные модули (не тесты)
    │   └── mod.rs
    ├── integration_test1.rs
    └── integration_test2.rs
...
...
...
```
### Ключевые особенности:

**Изоляция на уровне файлов**:
- Каждый `.rs` файл в `tests/` компилируется как самостоятельный исполняемый модуль
- Для каждого файла создается отдельный экземпляр кластера

### Создание кластера вручную

Picotest позволяет создавать и удалять кластер без использования макроса `#[picotest]`.

```rust
use rstest::rstest;

#[rstest]
fn test_without_picotest_macro() {
    let cluster = picotest::cluster(".", 0);
    assert!(cluster.path == ".");
}
```

## Модульное тестирование

Макрос `#[picotest_unit]` используется для написания юнит-тестов для плагинов, созданных с помощью утилиты [`pike`](https://github.com/picodata/pike).

### Требования к конфигурации

Для работы макроса `#[picotest_unit]` крейт должен быть объявлен как библиотека с поддержкой динамической линковки.

В `Cargo.toml` пакета необходимо указать:

```toml
[lib]
crate-type = ["cdylib"]
```

### Примеры использования:

```rust
#[picotest_unit]
fn test_my_http_query() {
    let http_client = fibreq::ClientBuilder::new().build();

    let http_request = http_client.get("http://example.com").unwrap();
    let http_response = http_request.send().unwrap();

    assert!(http_response.status() == http_types::StatusCode::Ok);
}
```

### Запуск тестов

Тесты запускаются через интерфейс cargo test:

```sh
cargo test
```

### Ограничения

1. `#[picotest_unit]` не может использоваться в модуле под `#[cfg(test)]`.

Пример **неверного** использования макроса:

```rust
#[cfg(test)]
mod tests {
    #[picotest_unit]
    fn test_declared_under_test_cfg() {}
}
```

Пример верного использования макроса:

```rust
mod tests {

    #[picotest_unit]
    fn test_is_NOT_declared_under_test_cfg() {}
}
```

По скольку каждый юнит-тест компилируется и линкуется в динамическую библиотеку плагина (см. [Структура плагина](https://docs.picodata.io/picodata/25.1/architecture/plugins/#structure)), он не должен быть задан в конфигурации, отличной от debug. В противном случае при сборке тестов они будут проигнорированы компилятором.

2. `#[picotest_unit]` не может использоваться совместно с другими [атрибутами](https://doc.rust-lang.org/rustc/tests/index.html#test-attributes).

Все атрибуты используемые совместно с макросом будут отброшены.

В примере ниже `#[should_panic]` будет отброшен в процессе компиляции.

```rust
#[should_panic]
#[picotest_unit]
fn test_will_ignore_should_panic_attribute() {}
```


## Подключение по Postrges протоколу

Picotest при запуске создаст дополнительного пользователя и назначит права на создание таблиц

```
User: Picotest
Password: Pic0test
```

Пример использования pgproto
```rust
use picotest::*;
use picotest_helpers::{PICOTEST_USER, PICOTEST_USER_PASSWORD};
use postgres::{Client, NoTls};

#[derive(Debug, PartialEq, Eq)]
struct User {
    id: i64,
    name: String,
    last_name: String,
}

#[picotest]
fn test_pg_connection() {
    let conn_string = format!(
        "host=localhost port={} user={} password={}",
        cluster.main().pg_port,
        PICOTEST_USER,
        PICOTEST_USER_PASSWORD
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
```

## Тестирование RPC ручек плагина

Для тестирования RPC ручек плагинов, предлагается использовать функцию `PicotestInstance::execute_rpc`, вызванную на конкретном инстансе, на котором задан RPC endpoint.

Аргументы функции:
 - plugin_name - имя плагина
 - path - имя эндпоинта, например ```/test```
 - service_name - имя сервиса, например ```main```
 - plugin_version - версия плагина, например ```0.1.0```
 - input - тело запроса, например структура ```User { ... }```

Тип тела запроса и возвращаемого значения определяется через шаблонные параметры.

Для тестов необходим асинхронный рантайм, поэтому необходимо указать макрос ```#[tokio::test]``` и добавить к самому тесту модификатор ```async```.

Пример теста:
```rust
#[derive(Serialize, Deserialize, Debug)]
pub struct User {
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExampleResponse {
    rpc_hello_response: String,
}

#[tokio::test]
#[picotest(path = "../tmp/test_plugin")]
async fn test_rpc_handle() {
    let user_to_send = User {
        name: "Dodo".to_string(),
    };

    let tnt_response = cluster
        .main()
        .execute_rpc::<User, ExampleResponse>(
            "test_plugin",
            "/greetings_rpc",
            "main",
            "0.1.0",
            &user_to_send,
        )
        .await
        .unwrap();

    assert_eq!(
        tnt_response.rpc_hello_response,
        "Hello Dodo, long time no see."
    );
}
```


## Покдлючение к admin консоле и выполнение sql/lua 

Для выполнения кода из консоли администратора на первом инстансе воспользуйтесь:

```rust
#[picotest]
fn test_query() {
    cluster.run_query("SELECT * FROM users"); // Выполнение SQL запроса
    cluster.run_lua("box.space.users:select()"); // Выполнение LUA кода
}
```

Если существует необходимость выполнить скрипт на любом другом инстансе: 

```rust
#[picotest]
fn test_run_query_on_instance() {
    cluster.instances[1].run_query("SELECT * FROM users");
    cluster.instances[2].run_lua("box.space.users:select()");
}
```


## Принудительная остановка инстанса

Метод `stop_instance` позволяет остановить конкретный экземпляр (instance) внутри кластера Picotest.


### Пример использования:
```rust
#[picotest]
fn test_stop_cluster_instance() {
    // получаем список всех инстансов в кластере
    let instances = cluster.instances();
    // останавливаем выбранный инстанс
    cluster.stop_instance(&instances[1])
}
```

