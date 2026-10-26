# Repository Agent Guide

This file is the canonical agent guide. `CLAUDE.md` is a symlink to `AGENTS.md` for compatibility with existing tooling.

## Development Commands

### Building and Testing
- `mise run build` - Build the debug binary with cargo
- `yt` - Run the debug build; `mise.toml` adds `target/debug` to `PATH`
- `mise run test` - Run tests with cargo-nextest
- `mise run snapshots` - Run the tests and accept changed insta snapshots
- `mise run release-dry-run` - Preview the version and changelog release-plz would propose for the last commit
- `mise run test-contract` - Run the contract tests against a real instance (`YT_CONTRACT_URL`, `YT_CONTRACT_TOKEN`, `YT_CONTRACT_PROJECT`; the project needs the default fields, versions 1.0 and 1.1, and time tracking with at least one work item type; the token must be allowed to create and delete a board)

### Code Quality
- `mise run lint` - Run all hk linters: rustfmt, clippy (`-D warnings`), pkl, jactionlint, final newlines, trailing whitespace
- `mise run lint-fix` - Run the linters and apply fixes
- `mise run deny` - Check dependencies for RustSec advisories and non-crates.io sources
- `mise run package` - Check that the crate packages and builds as crates.io would receive it
- `mise run ci` - Run build, lint, test, deny, and package; CI runs this on Linux and macOS
- `mise tasks` - List all available tasks

## Code Architecture

### High-Level Structure
`yt` is a Rust CLI for JetBrains YouTrack (crate `yt-cli`, binary `yt`).

Authentication supports several methods. Browser sign-in (OAuth) is first-class: `yt` opens a YouTrack link and the user approves access there, without creating or pasting a permanent token. Permanent tokens are supported alongside it.

Library modules (`src/`): `error` (YtError, exit codes), `env` (environment snapshot; the only place that reads the environment), `dirs`, `redact` (secret registry), `logger`, `output` (stdout: table, json, ndjson), `ui` (when yt may prompt; demand prompts), `instance` (instance URLs, issue IDs), `browser`, `config/` (settings, global config file, repo config, profile resolution), `credentials` (OS keyring), `http` (reqwest client, retry policy), `api/` (YouTrack client: projections own their `fields` selector, `paginate`, custom fields, issues, commands, projects, work items, articles, agiles (boards and sprints), comments, tags, links, attachments; comments and attachments take a `Parent`, an issue or an article), `auth/` (`discovery`, `pkce`, `oauth`, `loopback`, `token_source` credential sources, `login` sign-in flow), `fields/` (`schema`, `cache`, `change`, `resolve`, `plan`, `apply`: `-f` parsing, validation and planned writes), `apps/` (YouTrack apps that describe their API: `registry` of supported apps, `openapi` turns an app's OpenAPI document into commands, `request`, `cache` of documents), `editor` (`$VISUAL`/`$EDITOR`), `time`.

The binary (`src/cli/`) has one directory per command group and one file per command, or per pair such as `add` and `remove`. A command declares its arguments, calls library code and prints through `Ctx::output`; it holds no HTTP, file or keyring logic. `src/cli/shared/` holds run functions several command groups share (comments and attachments of issues and articles). `Ctx` (`src/cli/ctx.rs`) is passed to every command. The exception is `yt app <APP>`: `src/cli/app/` builds that app's commands at run time from its document (`tree.rs`) and parses them with usage-rs's runtime tables (`usage_rs::Parser`, `help::page`, `render_failure`).

### Configuration Files
- `mise.toml` - Tool versions, `[env]`, and tasks
- `mise.lock` - Per-platform download URLs and checksums for the tools in `mise.toml`
- `hk.pkl` - Linter definitions and the git `pre-commit` hook
- `deny.toml` - cargo-deny advisory and dependency source policy
- `clippy.toml` - Bans `std::env::var*` (read the environment through `yt::env::Env`), `std::process::exit` (return an `ExitCode`) and `eprintln!`/`eprint!` (use `yt::ui::eprint_line`)
- `.github/workflows/ci.yml` - Runs `mise run ci`; its `final` job is a required status check
- `.github/workflows/conventional-commits.yml` - Validates PR titles; its `title` job is a required status check
- `release-plz.toml` - release-plz settings: a release happens when the release PR is merged; `feat` bumps the minor version below 1.0; the GitHub Release starts as a draft
- `.github/workflows/release-plz.yml` - Opens the release PR and, when it is merged, tags, publishes to crates.io and creates the GitHub Release as a draft
- `.github/workflows/release.yml` - On a `v*` tag, builds the macOS and Linux binaries, attaches archives with completion files and `SHA256SUMS` to the draft release, then publishes it; published releases are immutable

### Build System
- One package with two targets: library `yt` (`src/lib.rs`, everything except the command line) and binary `yt` (`src/main.rs` and `src/cli/`). Library code never depends on `src/cli/`.
- Rust edition 2024
- `release` profile: LTO, stripped symbols, `codegen-units = 1`

## Development Guidelines

### Conventional Commits (REQUIRED)
PR titles MUST follow conventional commit format. Intermediate commit subjects SHOULD use the same format.

**Format:** `<type>[optional scope][optional !]: <description>`

**Types:**
- `feat:` - New features: commands, flags, output
- `fix:` - Bug fixes that change how `yt` behaves (not CI, docs or infrastructure)
- `refactor:` - Code changes that keep the behavior the same
- `docs:` - Documentation only: README, AGENTS.md, comments
- `style:` - Formatting only, no logic changes
- `perf:` - Performance improvements
- `test:` - Adding or changing tests only
- `chore:` - Maintenance: releases, dependency and tool updates
- `security:` - Security fixes and hardening
- `ci:` - CI workflows, release automation and repository tooling (`hk.pkl`, mise tasks)
- `revert:` - Reverting a previous change

`!` marks a breaking change; below 1.0 it bumps the minor version, as `feat` does.

**Scopes:**
- For command-specific changes, use the command group: `auth`, `profile`, `config`, `issue`, `project`, `time`, `article`, `board`, `api`, `completion`.
- For subsystem changes: `fields`, `output`, `http`, `credentials`, `release`, `deps`.

**Description Style:**
- Start with a lowercase character, or with an acronym such as `CLI` or `API`
- Use imperative mood ("add feature" not "added feature")
- Keep it concise but descriptive

**Examples:**
- `feat(issue): add --assignee to yt issue list`
- `fix(time): send loosely typed dates as YYYY-MM-DD`
- `fix(auth): refresh tokens under a lock`
- `docs: describe shell completion`
- `chore(deps): update reqwest to 0.13.2`
- `chore: release v0.1.0`
- `ci: pin the release workflow actions`

CI validates the PR title and re-runs when it is edited. It checks the type, the syntax, and that the description starts lowercase or with an acronym; imperative mood remains a review rule. Intermediate commit subjects are not checked.

### PR titles are release notes
Pull requests are squash-merged, so the PR title becomes the commit subject on `main`. release-plz builds `CHANGELOG.md` and the GitHub release notes from those subjects. Write the title for someone using `yt`: what changes for them, not how it was done. Releases are prepared by automation in a `chore: release vX.Y.Z` pull request; never change the version in `Cargo.toml`, edit `CHANGELOG.md`, or push tags by hand.

### Pre-commit Process
1. Run `mise run lint-fix` and `git add` any fixes before committing
2. Run `mise run ci` before declaring work done; it must pass

`hk install --mise` installs the `pre-commit` hook from `hk.pkl`. The hook runs the linters with auto-fix; `stash = "git"` preserves unstaged work.

### Tool Versions
- Pin every tool in `mise.toml` to an exact version (`hk = "2.5.0"`), never `latest` or a partial version.
- Upgrade with `mise upgrade --bump`, which keeps exact pins and updates `mise.lock`. Commit `mise.toml` and `mise.lock` together.
- When the `rust` pin changes, set `rust-version` in `Cargo.toml` to the same version.
- `min_version` in `mise.toml` and the `version` input of `jdx/mise-action` in every workflow that installs mise (`ci.yml`, `release.yml`, `release-plz.yml`) pin the same mise release. Change them together.

### Dependency Updates
- Add new dependencies at their latest release.
- Use the lowest-specificity requirement that expresses compatibility in `Cargo.toml`: `"1"` over `"1.2.3"`, and `"0.13"` over `"0.13.1"` for a pre-1.0 crate. `cargo add` writes the full version; shorten it.
- Routine updates should only change `Cargo.lock`. If the existing requirement accepts the target version, use `cargo update -p <crate> --precise <version>` instead of editing `Cargo.toml`.
- `Cargo.lock` is committed.

### GitHub Actions
- Pin every action to a full commit SHA followed by its version tag as a comment: `uses: owner/action@<sha> # v1.2.3`.
- Set `permissions: {}` at the workflow level and grant permissions per job.
- Check out with `persist-credentials: false`.

### Comments
- Write a comment only for a reason or constraint the code cannot show, in one short line.
- Do not write comments that restate what the code or config does.

### Testing
- Unit tests live in the same file as the code. Process tests in `tests/` run the built `yt` through `tests/common::TestEnv`, which isolates `HOME`, `YT_CONFIG_DIR`, `YT_CACHE_DIR`, `YT_STATE_DIR` and sets `YT_NO_INPUT=1`.
- Tests never use the OS keyring: library tests install `keyring_core::mock::Store`; process tests use profiles without a login or `YOUTRACK_URL`/`YOUTRACK_TOKEN`.
- `src/cli/snapshots/` holds the help-tree snapshot; any CLI change updates it with `mise run snapshots`.

## Important Implementation Notes

### Libraries
- CLI parsing: `usage-rs` v7 (https://usage.jdx.dev/rust/). Do not add clap.
- Interactive prompts: `demand` (https://github.com/jdx/demand).
- Do not use usage-rs `env = "…"` argument attributes: they read the variable with `std::env::var`, which `clippy.toml` bans, and treat an empty value as set. Read the variable through `Ctx::env` instead (`Env::get` treats an empty value as unset), as `Ctx::new` does for `YT_PROFILE`, `YT_FORMAT` and `YT_NO_INPUT`.

### YouTrack API
- Every response type implements `Projection` with the exact `fields` selector it decodes; never request without `fields`. App endpoints are the exception: they are an app's own API, so `yt app` sends them through `Client::raw` as the app's document describes.
- Reads are retried (network errors, 408, 429, 5xx): GET requests except file downloads, and read-only POSTs (`commands/assist`) through `Client::post_query`, which also do not count as writes. Writes (other POSTs, uploads, DELETE) are never retried.
- Read whole collections with `Client::get_all` or `Client::paginate`; one `$top` page is never the whole list.
- Command text writes multi-word values plainly (`State In Progress`); braces are search syntax only, and a command keeps them as part of the value (`tag {a b}` creates a tag named `{a b}`).
- Field changes go through `yt::fields`: single-value sets of bundle and user fields and `+=`/`-=` are commands (one request per change for all issues); multi-value sets, clears and typed values (dates, periods, numbers, text) are typed REST updates. Never join several changes into one command line.
- A work item type is sent by ID (`{"type": {"id": …}}`); YouTrack does not find it by name. `GET workItems` returns creation order and cannot sort, so `yt time list` reads every match and sorts by date.
- Articles are created with `{"project": {"shortName": …}, "summary", "content", "parentArticle": {"idReadable": …}}`; `"parentArticle": null` moves an article to the top level.
- Article IDs match the project part in any case, but YouTrack finds an article only with an uppercase `A` (`kb-A-12` works, `kb-a-12` does not); issue IDs match in any case. Sprint names are unique per board.
- Sprint issues: `POST agiles/{board}/sprints/{sprint}/issues` with `{"idReadable": …}` adds one; removing needs the issue's entity ID (`DELETE …/issues/{entity id}`), not its readable ID.
- Board columns come from `columnSettings.field` (any field, not only State); an issue's column is its value of that field.
- `+=` is the command `add NAME VALUE`; a bare `NAME VALUE` replaces every value of a multi-value field. `-=` is `remove NAME VALUE`.
- A command cannot create a tag whose name has spaces, so `yt issue tag add --no-validate` creates new tags with `POST /api/tags` before tagging.

### YouTrack apps
- `src/apps/registry.rs` is the allowlist: `yt app add` accepts only its ids. `list`, `ls`, `add`, `remove` and `help` are `yt app`'s own words, never app ids.
- yt calls only the app's own extension endpoints on the profile's instance: `/api/extensionEndpoints/<id>/…`, or `/api/{issues,admin/projects,articles,users}/{param}/extensionEndpoints/<id>/…` with a declared path parameter. Any other path in a document is skipped, and a path value is encoded as one segment.
- A document may list any subset of an app's operations, because an administrator can switch endpoints off. Whatever yt does not understand is skipped, never an error; only a document of another app, or one older than OpenAPI 3.1, is refused. An operation whose required parameter or body field cannot become an argument or flag is skipped as a whole.
- Operations that share `x-youtrack-command` are one command; yt uses the `global` one when it is listed.
- `403` with error `disabled`, or YouTrack's `404` "HTTP handler not found", makes yt fetch the document again. A GET is sent once more if the command now uses another endpoint; a POST is never sent twice. A command word missing from a cached document also fetches it once more, before anything is sent.
- App help and errors are rendered by usage-rs, coloured only when yt's own colour decision allows; server text in them was made printable when the document was read.
- Apps live only under `yt app`: `yt <unknown word>` keeps usage-rs's error, and while no app is enabled yt sends no app requests; `tests/app.rs` checks both.

### Credentials and sign-in
- Never commit credentials. Never print tokens in logs, error messages, or debug output; register every token with `yt::redact::register`.
- Stored credential types hold tokens as plain `String`s with a `Debug` that hides them, and the redactor filters logs and error output. `SecretString` is used only on the request path.
- Credentials live in the OS keyring (service `yt`, account `<login>@<host><context path>`, e.g. `jane@acme.youtrack.cloud`) as JSON: `{"kind":"token",…}` or `{"kind":"oauth",…}`. There is no file fallback.
- Browser sign-in uses, in order: `--client-id`, `YT_OAUTH_CLIENT_ID`, a client saved on a profile for the instance, then, where the instance allows automatic OAuth client registration (`client_id_metadata_document_supported`), `CIMD_CLIENT_ID`. Otherwise an administrator registers a client (Authorization Code + PKCE, no secret, redirect URI `http://127.0.0.1:<any port>/callback`).
- `CIMD_CLIENT_ID` is the URL of yt's OAuth Client ID Metadata Document, `oauth-client.json` on the `docs` branch, served by GitHub Pages. It is yt's identity on every instance: never move or rename the file. Its contents may change.
- `yt auth login --revocable` adds Hub's scope `0-0-0-0-0` so logout and re-login can delete refresh tokens; it is opt-in because users see it as "YouTrack Administration".
- Credentials never leave https for http: every client refuses a redirect from https to http, and discovery refuses http Hub, authorization and token endpoints for an https instance. Signing in to a plain-http instance on another machine warns.
- Refresh tokens last 90 days; refreshes run under a lock file in the state directory so concurrent yt processes never spend one twice.
- When a sign-in expires during a command, yt signs in again and re-runs the command only if the command has not yet sent a change (`Client::has_written`); otherwise it stops so changes are never applied twice.

### Untrusted input
- Server text (summaries, comments, names, error descriptions) reaches the terminal only through `Output`, `yt::ui::eprint_line`, `Ui` prompts or the error report, which replace control characters with `yt::output::printable`; JSON output escapes them instead.
- An app's OpenAPI document is server text too: `yt::apps::openapi` makes its summaries and descriptions printable before they reach help or completion, and drops enum choices with control characters, which would split completion output.
- A repository's `.yt.toml` comes from whoever wrote the repository: it may pick among the user's profiles and set the default project, but never suggests a sign-in URL.
- Files yt creates (downloads, editor buffers) are opened with `create_new`, so a symbolic link is never followed.

## GitHub Interactions

- `main` is the main branch.
- Develop features on feature branches created from `main`.
- Open pull requests against `main`; they are squash-merged.
- `docs` holds only the GitHub Pages site (the CIMD document); change it through pull requests against `docs`, which are squash-merged too.
