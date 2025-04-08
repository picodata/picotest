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
cargo add picotest
cargo add rstest
cargo add ctor
```

## Совместимость с Picodata

Picotest поддерживает версии Picodata, начиная с **25.1.1** и выше.

## Интеграционное тестирование

Макрос `#[picotest]` используется для написания интеграционных тестов и может применяться как к функциям, так и к модулям.

### Атрибуты макроса `#[picotest]`

Макрос `#[picotest]` поддерживает следующие аргументы:

- **path** - путь до директории плагина. Значение по умолчанию: `$(pwd)`.
- **timeout** - длительность ожидания после развертывания кластера и перед запуском тестов.
  Указывается в секундах. Значение по умолчанию: 0 секунд.

### Использование `#[picotest]` на функциях

При использовании макроса на функциях, `picotest` будет создавать кластер при каждом запуске очередного теста и удалять кластер по завершению теста.

```rust
use picotest::picotest;
use rstest::rstest;

#[picotest]
fn test_foo_bar() {
    assert_eq!("foo bar", "foo bar");
}
```

### Использование `#[picotest]` на модулях

При использовании макроса на модуле `picotest` автоматически пометит все функции, названия которых начинаются с `test_`, как `rstest`-функции. Кластер будет создан один раз и удален после выполнения всех тестов в модуле.

```rust
use picotest::picotest;

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

Макрос `#[picotest]` является оберткой над [`rstest`](https://github.com/la10736/rstest), поэтому поддерживает использование [`fixture`](https://docs.rs/rstest/latest/rstest/attr.fixture.html).

```rust
use picotest::picotest;

#[picotest]
mod test_mod {
    #[fixture]
    fn foo() -> String {
        "foo".to_string()
    }

    #[fixture]
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
}
```

### Запуск тестов

Запустите тесты с использованием переменной `RUST_TEST_THREADS=1`:

```sh
RUST_TEST_THREADS=1 cargo test
```

наличие переменной `RUST_TEST_THREADS=1` необходимо только в том случае, если вы используете несколько модулей или функций с макросом `#[picotest]`.

### Пользовательские хуки

Picotest поддерживает работу с хуками `before_all` и `after_all`
Для использования добавьте в свой `Cargo.toml` файл:

```toml
[dev-dependencies]
test-env-helpers = "0.2.2"
```

Пример:

```rust
use picotest::picotest;
use test_env_helpers::{after_all, before_all};

#[picotest]
#[before_all]
#[after_all]
mod test_mod {

    fn before_all() {
        todo!()
    }

    fn after_all() {
        todo!()
    }

    #[fixture]
    fn foo() -> String {
        "foo".to_string()
    }

    #[fixture]
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
}
```

### Создание кластера вручную

Picotest позволяет создавать и удалять кластер без использования макроса `#[picotest]`.

```rust
use rstest::rstest;

#[rstest]
fn test_without_picotest_macro() {
    let cluster = picotest::run_cluster(".", 0);
    assert!(cluster.is_ok());
    assert!(cluster.is_ok_and(|cluster| cluster.path == "."));
}
```

### Ограничения

1. Параллельное исполнение тестов не поддерживается. Тесты должны запускаться **последовательно**, т.е. с переменной окружения `RUST_TEST_THREADS=1` ([issue #2](https://github.com/picodata/picotest/issues/2))

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

Тесты запускаются через интерфейс cargo test с использованием переменной `RUST_TEST_THREADS=1`:

```sh
RUST_TEST_THREADS=1 cargo test
```

или опцией `--test-threads=1`:

```sh
cargo test -- --test-threads=1
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

3. Параллельное исполнение тестов не поддерживается. Тесты должны запускаться **последовательно**, т.е. с переменной окружения `RUST_TEST_THREADS=1` ([issue #2](https://github.com/picodata/picotest/issues/2))
