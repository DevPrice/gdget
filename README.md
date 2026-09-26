# gdget

gdget ("gadget") installs pinned Godot 4 addons into a project from a committed
manifest. Use it for addons that ship prebuilt native libraries, such as GDExtensions,
so you commit a small `addons.toml` file instead of the binaries.

gdget verifies every download against a pinned sha256 hash, caches archives per user,
and replaces addon folders atomically. It never touches folders it didn't install.

## Install

Download the archive for your platform from the
[releases page](https://github.com/DevPrice/gdget/releases), extract `gdget`, and add
it to your `PATH`.

Alternatively, if you have Rust 1.89 or later, install from
[crates.io](https://crates.io/crates/gdget):

```sh
cargo install gdget --locked
```

## Quick start

1.  In your Godot project folder (the one that contains `project.godot`), add an
    addon from the [Godot Asset Store](https://store.godotengine.org):

    ```sh
    gdget add PUBLISHER/ASSET
    ```

    Replace `PUBLISHER/ASSET` with the part of the asset's store address after
    `/asset/`, such as `devprice/godot-slang`. To add a specific release, append
    `@VERSION`, as in `devprice/godot-slang@v6.0.0`. To add an addon from any other
    site, pass the link to its zip instead.

    gdget downloads the addon, pins its hash in `addons.toml`, and installs it to
    `addons/NAME/`.

2.  Add the installed addon and gdget's working files to `.gitignore`:

    ```gitignore
    /addons/NAME/
    /.gdget/
    /addons.local.toml
    ```

3.  Commit `addons.toml` and `.gitignore`.

When someone else clones the project, they run `gdget sync` to install the pinned
addons. To sync automatically on checkout, see [Automate syncing](docs/automation.md).

## Documentation

- [Command reference](docs/commands.md): commands, flags, environment variables, and
  exit codes.
- [Manifest reference](docs/manifest.md): the `addons.toml` format and how gdget finds
  the addon folder in an archive.
- [Develop an addon locally](docs/local-development.md): use your local build instead
  of the pinned release.
- [Automate syncing](docs/automation.md): Git hooks and GitHub Actions.
- [Publish an addon for gdget](docs/publishing-addons.md): how to package a release.

## Build from source

Before you begin, install Rust 1.89 or later from [rustup.rs](https://rustup.rs).

1.  Clone the repository:

    ```sh
    git clone https://github.com/DevPrice/gdget.git
    cd gdget
    ```

2.  Build a release binary:

    ```sh
    cargo build --release
    ```

    The binary is `target/release/gdget` (`gdget.exe` on Windows).

3.  Run the tests and lints that CI runs:

    ```sh
    cargo test
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check
    ```

    The tests use a local HTTP server and don't need network access.

## License

gdget is licensed under the [MIT License](LICENSE).
