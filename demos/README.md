# Demos

| Project | Characteristics |
| --- | --- |
| [minimal](minimal/README.md) | One Cargo package with a library and one firmware binary. |
| [multi-firmware](multi-firmware/README.md) | Two firmware packages sharing a library and platform in one workspace. |
| [multi-workspace](multi-workspace/README.md) | Buzzer, screen and two LED firmwares across three workspaces, with shared libraries and a two-bin LED package. |

## Common commands

Run from the repository root. Set `project` to one of the directories listed above.

```sh
project=demos/minimal
just run --project-root "$project" info
just run --project-root "$project" verify --all-firmwares
just run --project-root "$project" build --all-firmwares --release
```

## Guides

- [Usage](../docs/usage.md)
- [Project configuration](../docs/configuration.md)
- [Development and demo project tests](../docs/development.md#maintain-the-examples)
