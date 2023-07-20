# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0] - 2023-07-20

### Changed

- Update to `bevy` 0.11.
- Mappable network events now need to implement `MapEventEntities` instead of `MapEntities`.

### Removed

- `ClientState` and `ServerState`, use conditions from `bevy_renet` and `resource_added()` / `resource_exists()` / `resource_removed()`.
- `ServerSet::Authority`, use `has_authority()` instead.

## [0.6.1] - 2023-07-09

### Changed

- Update `ParentSync` in `CoreSet::PostUpdate` to avoid one frame delay.

## [0.6.0] - 2023-07-08

### Added

- `SendPolicy` added to API of event-creation for user control of delivery guarantee (reliability and ordering).

### Changed

- `ParentSync` no longer accepts parent entity and just synchronizes hierarchy automatically if present.

## [0.5.0] - 2023-06-26

### Added

- `ServerSet::ReceiveEvent` and `ServerSet::SendEvent` for more fine-grained control of scheduling for event handling.

### Changed

- Update server to use `TickPolicy` instead of requiring a tick rate.

### Fixed

- Unspecified system ordering could cause tick acks to be ordered on the wrong side of world diff handling.
- Crash after adding events without `ServerPlugin` or `ClientPlugin`.

## [0.4.0] - 2023-05-26

### Changed

- Swap `registry` and `event` arguments in `BuildEventSerializer` for consistency with `ReflectSerializer`.
- Update to `bevy_renet` 0.0.12.

## [0.3.0] - 2023-04-15

### Added

- Support for sending events that contains `Box<dyn Reflect>` via custom serialization implementation.

### Changed

- Accept receiving system in `add_client_event_with` and sending system in `add_server_event_with`.
- Make `EventChannel<T>` public.

## [0.2.3] - 2023-04-09

### Fixed

- Panic that could occur when deleting `RenetServer` or `RenetClient` resources.

## [0.2.2] - 2023-04-05

### Fixed

- Do not panic if an entity was already despawned on client.

## [0.2.1] - 2023-04-02

### Fixed

- Incorrect last tick detection.

## [0.2.0] - 2023-04-01

### Changed

- Use `#[reflect(MapEntities)]` from Bevy 0.10.1 instead of custom `#[reflect(MapEntity)]`.

### Fixed

- Tick checks after overflow.

## [0.1.0] - 2023-03-28

Initial release after separation from [lifescape](https://github.com/lifescapegame/lifescape).

[unreleased]: https://github.com/lifescapegame/bevy_replicon/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/lifescapegame/bevy_replicon/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/lifescapegame/bevy_replicon/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/lifescapegame/bevy_replicon/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/lifescapegame/bevy_replicon/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/lifescapegame/bevy_replicon/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/lifescapegame/bevy_replicon/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/lifescapegame/bevy_replicon/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/lifescapegame/bevy_replicon/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/lifescapegame/bevy_replicon/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/lifescapegame/bevy_replicon/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/lifescapegame/bevy_replicon/releases/tag/v0.1.0
