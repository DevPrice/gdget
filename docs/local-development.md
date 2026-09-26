# Develop an addon locally

While you work on an addon, you can have a game use your local build instead of the
pinned release. gdget links `addons/NAME/` to your build folder, so each rebuild takes
effect in the game without syncing again.

Overrides live in `addons.local.toml` in the project folder. The file is personal to
your machine; don't commit it.

## Before you begin

- Add `/addons.local.toml` to the project's `.gitignore`.
- Make sure the project has an `addons.toml` file. It can be empty.

## Use a local build

1.  Create `addons.local.toml` in the project folder:

    ```toml
    [overrides]
    NAME = "PATH"
    ```

    Replace the following:

    - `NAME`: the addon's install folder name.
    - `PATH`: the path to your build, relative to the project folder. It can be the
      addon folder itself, such as `../godot-slang/addons/godot-slang`, or a folder laid
      out like the release zip. gdget finds the addon folder with the
      [same rules](manifest.md#how-gdget-finds-the-addon-folder) it uses for zips.

    The addon doesn't need to be in `addons.toml`, so you can develop an addon before
    its first release.

2.  Run `gdget sync`.

    gdget replaces `addons/NAME/` with a link to your build. On Windows, the link is a
    directory junction, which doesn't need administrator rights or Developer Mode.
    Elsewhere, it's a symbolic link.

3.  To confirm the override, run `gdget status`. The addon shows as
    `overridden -> PATH`.

## Return to the pinned release

1.  Remove the addon's line from `addons.local.toml`, or delete the file.
2.  Run `gdget sync`.

gdget removes the link and installs the pinned release. It never deletes or changes your
build folder.
