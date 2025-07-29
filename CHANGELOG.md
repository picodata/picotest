# Change Log

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/) and this project adheres to [Semantic Versioning](http://semver.org/).


## [1.7.1]

### Fixed

* [Unit] fix dylib name to match picodata cluster name


## [1.7.0]

### Fixed

* Added the ability to get a list of instances by level name


## [1.6.2]

### Fixed

* Fixed name conflict in unit tests


## [1.6.1]

### Changed

* Removal of postfix/suffix in the output returned from picodata admin after Lua query execution


## [1.6.0]

### Added

* Run all migrations on default tier when unit tests are used


## [1.5.0]

### Added

* Add the ability to execute lua/sql code on any instance

### Changed

* Bump pike version from 2.4.5 to 2.5.0


## [1.4.1]

### Changed

* Bump pike version from 2.4.4 to 2.4.5


## [1.4.0]

### Added

* Description for using plugin config
* `PluginConfigMap` type for external configuration in `apply_config` 
* Added support for `picodata 25.1.2`

## [1.3.1]

### Fixed

* Fix users grant
* Save logs after tests running

## [1.3.0]

### Added

* Implement rpc calls in tests
* Run all unit-tests on a single-node cluster with default tier

## [1.2.1]

### Fixed

* Published PICOTEST_USER, PICOTEST_USER_PASSWORD variable

## [1.2.0]

### Added

* Abitity to get picodata instance pg_port
* Picotest user for testing

## [1.1.0]

### Added

* Automatic plugin root discovery.
* Cluster as a fixture.
* Automatic addition of the cluster fixture when using the `#[picotest]` macro.
* Ability to use the `#[case]` attribute.

### Fixed

* Cluster now stops correctly regardless of the number of tests being run.
* Parallel test execution is now enabled without the strict requirement of specifying `test-threads = 1`.

## [1.0.0]

This is the first public release of the project.
