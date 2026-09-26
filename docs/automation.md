# Automate syncing

This page shows how to run `gdget sync` automatically after Git checkouts and in GitHub
Actions.

## Sync after checkouts and pulls

Git hooks can run `gdget sync` after you switch branches or pull.

1.  In your project repository, create `.githooks/post-checkout` with the following
    content:

    ```sh
    #!/bin/sh
    # post-checkout passes 0 as $3 for file checkouts; only sync on branch switches.
    [ "$3" = "0" ] && exit 0
    if ! command -v gdget >/dev/null 2>&1; then
      echo "gdget not found; skipping addon sync" >&2
      exit 0
    fi
    gdget sync || echo "gdget sync failed; fix the problem above and run it again" >&2
    ```

2.  Copy the file to `.githooks/post-merge`.

3.  On macOS and Linux, make both files executable:

    ```sh
    chmod +x .githooks/post-checkout .githooks/post-merge
    ```

4.  Commit the `.githooks` folder.

5.  In each clone, tell Git to use the hooks:

    ```sh
    git config core.hooksPath .githooks
    ```

Git ignores the exit status of these hooks, so a failed sync prints an error but never
blocks a checkout.

## Sync in GitHub Actions

Add these steps to your workflow before the step that exports the game:

```yaml
- name: Install gdget
  shell: bash
  run: |
    curl -fsSL https://github.com/DevPrice/gdget/releases/download/v0.1.0/gdget-x86_64-unknown-linux-musl.tar.gz \
      | tar -xz -C "$RUNNER_TEMP"
    echo "$RUNNER_TEMP" >> "$GITHUB_PATH"

- name: Cache addon archives
  uses: actions/cache@v4
  with:
    path: ~/.cache/gdget
    key: gdget-${{ hashFiles('addons.toml') }}
    restore-keys: gdget-

- name: Install Godot addons
  run: gdget sync
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

`GITHUB_TOKEN` is optional for public releases, but it raises GitHub's rate limits. In
GitHub Actions, gdget reports warnings and errors as workflow annotations.

To fail a job when the installed addons don't match the manifest, without changing
them, run `gdget sync --check` instead.
