# Develop an addon locally

While you work on an addon, you can have a game use your local build instead of the
pinned release. gdget links `addons/NAME/` to your build folder, so each rebuild takes
effect in the game without syncing again.

gdget records these overrides in `addons.local.toml` in the project folder. The file is
personal to your machine; don't commit it.

## Before you begin

Add `/addons.local.toml` to the project's `.gitignore`.

## Use a local build

1.  In the project folder, run:

    ```sh
    gdget link PATH
    ```

    Replace `PATH` with the path to your build. It can be the addon folder itself, such
    as `../godot-slang/addons/godot-slang`, or a folder laid out like the release zip,
    such as `../godot-slang`. gdget finds the addon folder with the
    [same rules](manifest.md#how-gdget-finds-the-addon-folder) it uses for zips.

    gdget names the override after the build's folder under `addons/`, or after the
    folder itself if it's the addon folder. If gdget can't tell, or to use a different
    name, pass the name first: `gdget link NAME PATH`.

    gdget replaces `addons/NAME/` with a link to your build. On Windows, the link is a
    directory junction, which doesn't need administrator rights or Developer Mode.
    Elsewhere, it's a symbolic link.

    The addon doesn't need to be in `addons.toml`, and the project doesn't need an
    `addons.toml` at all, so you can develop an addon before its first release.

2.  To confirm the override, run `gdget status`. The addon shows as
    `overridden -> PATH`.

## Return to the pinned release

Run:

```sh
gdget unlink NAME
```

gdget removes the link and installs the pinned release, if there is one. It never
deletes or changes your build folder.

## Edit overrides by hand

`gdget link` and `gdget unlink` edit `addons.local.toml` for you. You can also edit it
yourself and then run `gdget sync`:

```toml
[overrides]
godot-slang = "../godot-slang/addons/godot-slang"
```

Each entry maps an install folder name to a build folder. Relative paths are relative to
the project folder.
