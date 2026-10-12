# yt

A command line interface for [JetBrains YouTrack](https://www.jetbrains.com/youtrack/),
written in Rust. Not affiliated with JetBrains.

> **Status:** early releases (0.x). Covers sign-in, profiles, settings, issues (search, show, create, edit, commands, comments, tags, links, attachments), projects, time tracking, articles, boards and sprints, and `yt api`. Commands and output may change before 1.0.

## Install

`yt` runs on macOS and Linux; Windows is not supported. Prebuilt binaries cover macOS (Apple silicon and Intel) and Linux (x86_64 and aarch64, statically linked).

```sh
mise use -g github:vaivars/rust-yt-cli   # prebuilt binary with mise
cargo binstall yt-cli                    # prebuilt binary with cargo-binstall
cargo install --locked yt-cli            # build from source (Rust 1.99 or newer)
```

Or download the archive for your platform and `SHA256SUMS` from the [releases page](https://github.com/vaivars/rust-yt-cli/releases), verify the checksum, and put `yt` on your PATH:

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing   # Linux: sha256sum -c --ignore-missing SHA256SUMS
tar -xzf yt-v0.0.1-aarch64-apple-darwin.tar.gz
mv yt-v0.0.1-aarch64-apple-darwin/yt ~/.local/bin/
```

### Shell completion

Each archive contains `completions/yt.bash`, `completions/_yt` (zsh) and `completions/yt.fish`. Copy the one for your shell to where it loads completions, or generate it:

```sh
yt completion bash > ~/.local/share/bash-completion/completions/yt
yt completion zsh > "${fpath[1]}/_yt"
yt completion fish > ~/.config/fish/completions/yt.fish
```

### macOS

The binaries are not signed by Apple. A browser download is quarantined; clear it with `xattr -d com.apple.quarantine ~/.local/bin/yt`. Installs with mise, cargo or `curl` are not affected. `yt` keeps your sign-in in the Keychain, and after each update macOS asks once whether the new `yt` may read it.

## Sign in

```sh
yt auth login https://acme.youtrack.cloud
```

- **Browser sign-in (OAuth):** `yt` opens YouTrack in your browser and you approve access there, with no token to create or copy. It needs an OAuth client that a YouTrack administrator registers once for the instance (Authorization Code with PKCE, no client secret, redirect URI `http://127.0.0.1:<any port>/callback`); `yt` explains the setup when the client is missing. Pass its ID with `--client-id` or `YT_OAUTH_CLIENT_ID`. Over SSH, or with `--no-browser`, `yt` prints the sign-in address instead: open it on any device, approve, and paste back the address the browser lands on.
- **Permanent token:** create one in your YouTrack profile and sign in with `--method token`, or pipe it in with `--token-stdin`.
- **Scripts and CI:** set `YOUTRACK_URL` and `YOUTRACK_TOKEN` instead of signing in. `YOUTRACK_TOKEN` alone replaces the stored sign-in of the profile in use.

Sign-ins are kept in the OS keyring: the macOS Keychain, or the Secret Service on Linux (GNOME Keyring, KWallet), which must be running. `yt auth status` shows the current sign-in.

## Usage

```sh
yt issue list 'for: me #Unresolved'
yt issue show WEB-42
yt issue list -p WEB --format json      # for scripts
yt issue create -p WEB -s 'Login fails' -f Priority=Major
yt issue edit WEB-42 -f 'State=In Progress' -f Assignee=me
yt issue edit WEB-42 -f 'Fix versions+=1.2' -f Assignee=   # add a version, unassign
yt issue comment add WEB-42 -b 'Fixed in main'
yt issue attachment upload WEB-42 screenshot.png
yt time log WEB-42 '1h 30m' --type Development -b 'Login fix'
yt time list --from 2026-10-01
yt article list -p KB --tree
yt board show 'Web Kanban Board'
yt api 'admin/projects?$top=5' --fields shortName,name
```

Every command has `--help`. Commands that print data also take `--format json` or `--format ndjson` (or `YT_FORMAT`) for scripts.

## Configuration

Each sign-in is a profile in `~/.config/yt/config.toml` (or `$XDG_CONFIG_HOME/yt`, or `YT_CONFIG_DIR`). With several instances, pick one with `--profile` or `YT_PROFILE`, or set the default with `yt profile use`.

A repository can pin its instance and default project in `.yt.toml` or `.config/yt.toml`:

```toml
url = "https://acme.youtrack.cloud"
project = "WEB"
```

`yt config list` shows the settings (`http_timeout`, `http_retries`, `color`, `cache_ttl`) and where each value comes from; `yt config set` changes one, and `YT_<NAME>` overrides it. `YT_LOG=debug` or `-v` shows what `yt` does. `--no-input` (or `YT_NO_INPUT=1`) makes `yt` fail instead of prompting, and `-y` confirms destructive actions.

Exit codes: 0 success, 1 error, 2 usage error or missing input, 4 not signed in or sign-in rejected, 130 cancelled.

## Development

All tooling is managed by [mise](https://mise.jdx.dev), pinned in `mise.toml` and `mise.lock`.

```sh
mise install          # Rust toolchain + dev tools
hk install --mise     # git hooks (fmt, clippy, workflow lint) on commit
mise run ci           # everything CI runs
```

| Task                       | What it does                                          |
| -------------------------- | ----------------------------------------------------- |
| `mise run build`           | `cargo build`; `yt` on your PATH points at the result |
| `mise run test`            | `cargo nextest run`                                   |
| `mise run snapshots`       | the tests, accepting changed insta snapshots          |
| `mise run lint`            | all linters from `hk.pkl`                             |
| `mise run lint-fix`        | same, applying fixes                                  |
| `mise run deny`            | `cargo deny` advisories and sources check             |
| `mise run package`         | `cargo package`: the crate builds as published        |
| `mise run release-dry-run` | the version and changelog release-plz would propose   |
| `mise run ci`              | build, lint, test, deny, package                      |

See [AGENTS.md](AGENTS.md) for the conventions: commit types, tool pins and code layout.

## License

[MIT](LICENSE)
