# Change Log

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/) and this project adheres to [Semantic Versioning](http://semver.org/).

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
