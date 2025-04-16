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

Picotest поддерживает версии Picodata, начиная с **25.1.1** и выше.

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
| `path`    | путь до директории плагина | Current directory |
| `timeout` | Таймаут перед запуском первого теста (seconds) | 5 |

# Управление кластером в Picotest

## Жизненный цикл и изоляция кластера

Picotest обеспечивает полную изоляцию тестовых окружений за счет автоматического управления жизненным циклом кластера:

### Архитектура тестирования

```bash
my_plugin/
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
