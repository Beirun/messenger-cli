# messenger-cli

An unofficial command-line client for a personal Facebook Messenger account, and the library
that [`messenger-mcp`](https://github.com/Beirun/messenger-mcp) is built on.

It talks to Messenger the way the web client does: one WebSocket connection to Meta's LightSpeed
gateway handles listing chats, reading history and sending messages, using the cookies of a
signed-in session. One-to-one chats are end-to-end encrypted, and Meta drops plaintext messages
sent to them, so those go through a browser backend that drives a headless Chrome profile and
lets the Messenger web client do the encryption.

## Commands

| Command | Description |
| --- | --- |
| `messenger login` | Open Chrome, wait for sign-in, save the session cookies. |
| `messenger status` | Load the saved cookies and check the session against Messenger. |
| `messenger chats` | List recent chats, newest first. |
| `messenger read <chat_id>` | Read the newest messages of a chat. |
| `messenger send <chat_id> <text>` | Send a text message. Use `-` as the text to read it from stdin. |
| `messenger react <chat_id> <message_id> <emoji>` | React to a message in a group chat. |
| `messenger attach <chat_id> <path>` | Send a file, image, video or audio, with `-m/--caption`. |

`chats` and `read` take `-n/--limit`, between 1 and 100. `read` and `send` take
`--backend gateway|browser`. Every command takes `--json` for machine-readable output. `read`
prints the message ids that `react` takes.

```sh
messenger chats -n 10
messenger read 5986541808137198 --backend browser
echo "running late" | messenger send 5986541808137198 -
```

## Install

Rust 1.100 or newer through rustup. `rust-toolchain.toml` pins the exact version.

```sh
cargo build --release      # target/release/messenger
cargo install --path .     # or put it on PATH
```

Chrome or Chromium is needed for `login` and for `--backend browser`. The binary is looked up in
`MESSENGER_CHROME`, then in an agent-browser install, then on `PATH`. The gateway commands never
launch a browser. The built binary links against glibc, `libm` and `libgcc` only.

## Sign in

```sh
messenger login
```

Chrome opens at messenger.com and waits for you to sign in, then writes the cookies with `600`
permissions and closes the window. It waits 300 seconds by default; set `MESSENGER_LOGIN_TIMEOUT`
to wait longer.

- If Messenger asks for your PIN to restore encrypted chats, enter it in that window. The profile
  keeps the encrypted state, so the browser backend does not ask again.
- Chrome for Testing rejects Google sign-in with "This browser or app may not be secure". Use the
  email and password form, or point `MESSENGER_CHROME` at a regular Chrome install.
- Only one process may use the profile at a time. A stale `SingletonLock` makes the browser
  commands refuse to start until the Chrome using it is closed.

## Backends

`gateway` is the default and the faster path. It reads group chats and sends to them, but Meta
drops plaintext sends to encrypted one-to-one chats, so those appear to succeed and never arrive.

`browser` handles encrypted chats. It opens the thread in headless Chrome, waits for the message
list to stop changing, and reads the rendered messages. Sends type into the composer, reload the
thread, and report success only if the message appears afterwards. Reads return only what is
currently rendered, so long histories are partial, and opening a chat can mark it read.

Reactions only exist on the gateway path, so they work in group chats and not in encrypted
one-to-one chats. Attachments take that route too: the file is uploaded to Meta's mercury
endpoint and the message refers to the upload id, so encrypted one-to-one chats have no
attachment path yet either.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `MESSENGER_COOKIES` | `~/.config/messenger-mcp/cookies.json` | Where `login` saves cookies and every command reads them. JSON export and Netscape `cookies.txt` are both accepted. |
| `MESSENGER_CHROME` | agent-browser's Chrome, then system Chrome | Chrome binary used by `login` and the browser backend. |
| `MESSENGER_LOGIN_TIMEOUT` | `300` | Seconds `login` waits for sign-in. |

The state paths keep the `messenger-mcp` name because `messenger-mcp` shares this session,
cookies and Chrome profile. Changing the path means signing in again.

## Use as a library

The modules are public, and `messenger-mcp` depends on them for its MCP tools:

```rust
use messenger_cli::{auth::CookieJar, browser, client::Client};

let jar = CookieJar::load(&messenger_cli::auth::cookies_path()?)?;
let threads = Client::new(jar)?.list_threads().await?;
```

`client::Client` covers the gateway (`check_session`, `list_threads`, `read_messages`,
`send_text`), and `browser::{read_messages, send_message}` cover encrypted chats. Both return
plain structs, with no formatting.

## Disclaimer

Unofficial, not affiliated with Meta. Automating a personal account breaks Messenger's terms and
can get the account restricted or banned. Sending from a shell has no undo, which is why `send`
asks for confirmation unless you pass `--yes`.

## License

AGPL-3.0-only. The LightSpeed protocol client is
[`messenger-client`](https://github.com/rgryta/messenger-sync), which is AGPL-3.0 because it was
ported from mautrix-meta, so this project carries the same license.
