# Foundation

**Foundation** is an independent Final Fantasy XIV housing simulator fork built on top of [Kawari](https://codeberg.org/redstrate/Kawari). It focuses on local housing previewing, indoor/outdoor estate editing, snapshots/undo, ReMakePlace JSON import/export, and bidirectional real-time editing workflows.

Foundation currently targets patch **7.58** (`2026.11.03.0000.0000`) on Windows clients.

## Upstream Attribution & Non-Affiliation Notice

- Foundation is a downstream modified fork (2026) of **[Kawari](https://codeberg.org/redstrate/Kawari)** created by **[redstrate](https://redstrate.com/)** and the Kawari contributors.
- **Foundation is an independent fork and is not affiliated with, endorsed by, or supported by redstrate or the upstream Kawari project.** Please do **not** open bug reports, feature requests, or pull requests for Foundation in upstream Kawari repositories.
- Downstream housing simulator extensions, tooling, and documentation in this fork include AI-assisted software engineering work. They are intentionally maintained downstream out of respect for upstream Kawari's contributor guidelines and preferences.
- **Core Scope Disclaimer (from Kawari)**: This software is **not** a private server meant for playing retail game content. It only implements what is needed for its local simulator/research scope. If you want to actually play Final Fantasy XIV, create a free trial or official service account.

## What Foundation Adds on Top of Kawari

Most housing simulator extensions are isolated under [`servers/world/src/housing_sim/`](servers/world/src/housing_sim/) to keep the diff against upstream Kawari clean and minimal:

- **Housing Simulation & Protocol Handling**: Multi-estate/apartment creation, warp/entry state handling, furniture placement/movement/rotation/dyeing, indoor/outdoor layout switching, and appearance updates.
- **Slot-Preserving Live Diff Engine**: Incremental live packet updates for ReMakePlace edits with snapshot creation and undo history.
- **Housing Studio HTTP & SSE API (`:21064/api/v1/housing/*`)**: REST endpoints for estate inspection, preset validation/application, snapshot timeline, ReMakePlace JSON import/export, and realtime Server-Sent Events (`/events`) — plus a lightweight browser fallback UI served at `http://127.0.0.1:21064/`.
- **In-Game `!housing` Commands**: Chat/Lua helper commands for creating, entering, reloading, resetting, and exporting estates.
- **Companion Tools**:
  - **[Foundation-Studio](https://github.com/bloooowfish/Foundation-Studio)**: Unified Tauri v2 Windows desktop launcher, local server controller, watched preset folder manager, and embedded Housing Studio UI.
  - **[rmp-bridge](https://github.com/bloooowfish/rmp-bridge)**: Zero-touch shadow ReMakePlace runtime and bidirectional live sync bridge.

## Quick Start

1. Prepare `config.yaml` in the repository root and place `oodle-network-shared.dll` in `oodle/` (or the target profile directory).
2. Build and run with the `kawari/oodle` feature enabled:

```powershell
cargo build --workspace --exclude kawari-navimesh --features kawari/oodle
.\run-server.cmd
```

Once running:
- **Account / Setup Web UI**: `http://127.0.0.1:21062/setup`
- **Housing API & Browser Studio Fallback**: `http://127.0.0.1:21064/`
- Or launch **[Foundation Studio](https://github.com/bloooowfish/Foundation-Studio)** (`foundation-studio.exe`) to manage the server, client login, ReMakePlace sync, and Housing Studio in one window.

Upstream Kawari setup fundamentals remain documented at [docs.xiv.zone/kawari](https://docs.xiv.zone/kawari/setup/overview.html).

## Credits & Thank You

This fork would not exist without the engineering work of upstream Kawari and the broader FFXIV open-source research community:

- **[Kawari](https://codeberg.org/redstrate/Kawari)**, **[physis](https://github.com/redstrate/physis)**, and **[icarus](https://github.com/redstrate/icarus)** by **redstrate** and Kawari contributors — for the modern, clean Rust FFXIV server foundation and Excel data libraries that make downstream housing research possible.
- **Original Kawari Credits**:
  - [Sapphire](https://github.com/SapphireServer/Sapphire) for reference.
  - [iolite](https://github.com/0xbbadbeef/iolite) for inspiration & reference.
  - [TemporalStasis](https://github.com/NotNite/TemporalStasis) for tooling and reference.
  - [Project Chronofoil](https://github.com/ProjectChronofoil/) for easy packet capture.
  - [FFXIVClientStructs](https://github.com/aers/FFXIVClientStructs/) for being an invaluable resource for the client's internals.
- **Housing Tooling Ecosystem**:
  - **jawslouis**, original creator of MakePlace.
  - **[ReMakePlace](https://github.com/RemakePlace/app)** and **[ReMakePlace Plugin](https://github.com/RemakePlace/plugin)** maintainers for preserving the community housing layout format and editor.

## License

Foundation is licensed under the **[GNU Affero General Public License v3 (AGPL-3.0)](LICENSE)**, continuing Kawari's license. All source code in this fork—including downstream housing simulator additions—is publicly available under the terms of AGPL-3.0. Prebuilt Windows release archives include `THIRD_PARTY_NOTICES.txt` covering statically linked Rust crates, bundled Bootstrap v5.3.8 (`MIT`), vendored Lua 5.4 (`MIT`), and bundled SQLite (`Public Domain`).

*FINAL FANTASY* and *FINAL FANTASY XIV* are registered trademarks of Square Enix Co., Ltd. Foundation is an independent, non-commercial simulator/research fork and is not affiliated with, endorsed by, or supported by Square Enix. Proprietary or retail files (such as `oodle-network-shared.dll` from RAD Game Tools / Epic Games — subject to the [Unreal Engine EULA](https://www.unrealengine.com/eula/unreal) when obtained via Unreal Engine tooling — and FINAL FANTASY XIV client data) are never distributed in this repository or its release archives.
