# sndbx

A lightweight sandbox for coding agents: one binary that runs an agent's docker image with your project mounted and its config kept between runs. No daemon, VM or runtime; it puts together a `docker run`, logs it and runs it.

- **Just docker:** sandboxes are plain containers, so `docker ps`, `logs` and `exec` work. `sndbx logs commands` shows what sndbx ran, so you can [run it yourself](#run-without-sndbx).
- **Small:** one Rust binary, five dependencies, one YAML file. The [dashboard](#dashboard) is an optional second binary.
- **Any image:** an entry is a name and an optional tag.
- **Optional egress control:** a squid proxy limits sandboxes to the hosts you list, or logs everything they reach.

The repo has Dockerfiles for Claude Code, opencode, Codex, Cursor and dsh, see [Images](#images).

## Install

Download the binary for your platform from the [latest release](https://github.com/ivanouski/sndbx/releases/latest) into a directory in `PATH`:

```sh
asset=sndbx-macos   # or sndbx-linux-x86_64, sndbx-linux-arm64
curl -fL --create-dirs -o ~/.local/bin/sndbx "https://github.com/ivanouski/sndbx/releases/latest/download/$asset" && chmod +x ~/.local/bin/sndbx
```

For the [dashboard](#dashboard), `sndbx top`, download `sndbx-top` next to it:

```sh
curl -fL -o ~/.local/bin/sndbx-top "https://github.com/ivanouski/sndbx/releases/latest/download/sndbx-top-${asset#sndbx-}" && chmod +x ~/.local/bin/sndbx-top
```

To build the images with `sndbx build`, clone this repo and set `images_dir` to its `images/` in the settings, see [Configure](#configure).

Or build sndbx from a clone with a recent Rust toolchain, e.g. [mise](https://mise.jdx.dev)'s. It installs into `~/.cargo/bin` and finds `images/` without any setting:

```sh
cargo install --path .
cargo install --path top   # optional, the dashboard
```

Optionally, install shell completion. Arguments after `--` complete with docker's completion; for zsh, if docker's is installed.

```sh
# bash 4.0+, with bash-completion
cp completions/bash/sndbx ~/.local/share/bash-completion/completions/sndbx
# or, without it, in ~/.bashrc
source ~/src/sndbx/completions/bash/sndbx

# zsh, in ~/.zshrc before compinit, or copy completions/zsh/_sndbx to a directory in fpath
fpath=(~/src/sndbx/completions/zsh $fpath)
```

## Use

```sh
sndbx run claude                                      # mount the current directory
sndbx run claude -v ../api -v ../web                  # mount several, start in the last one
sndbx run claude -v ../api -v ../web -w api           # start in api
sndbx run claude -v . -v ../notes                     # a plain -v replaces the current directory, add . to keep it
sndbx run claude -v ./sdk:/opt/sdk:ro                 # mount a read-only SDK, start in the current directory
sndbx run claude -p 3000 -p 8080:80                   # publish ports, 3000 means 3000:3000
sndbx run claude -k                                   # keep the container after it exits, for resume
sndbx run claude -- --memory=8g -e FOO=bar            # pass extra arguments to docker run
sndbx exec claude my-project bash                     # run a command in a running sandbox
sndbx resume claude my-project-1                      # restart a stopped sandbox
sndbx list configured                                 # image names in the settings
sndbx list buildable                                  # image names build accepts
sndbx list running claude                             # claude's sandboxes; stopped for the stopped ones
sndbx logs commands                                   # follow the docker commands sndbx ran
sndbx logs proxy                                      # follow the proxy's requests
sndbx top                                             # a dashboard of the sandboxes, the proxy and the images
```

`sndbx run` also:

- builds the image if it's missing and `images_dir` has a Dockerfile for it; pull or build other images yourself
- mounts `-v` paths without a destination, and the current directory, at `/workspace/<basename>`
- mounts the image's config directory at `<config_target>` in the container user's home
- passes `COLORTERM`, `TERM_PROGRAM` and `TERM_PROGRAM_VERSION` from the host
- labels the container `sndbx.image=<name>`, which `exec`, `resume` and `list` find it by
- names the container after the starting directory, `my-project`, then `my-project-1`, … Other characters than letters, digits, `_` and `-` become `-`. Override with `-- --name=<name>`.

Docker commands are logged to `~/.local/state/sndbx/sndbx.log` (`$XDG_STATE_HOME/sndbx/` if set).

## Configure

Settings live in `~/.config/sndbx/settings.yml` (`$XDG_CONFIG_HOME/sndbx/` if set). `run` creates an example, and `build` adds the images it builds. Relative paths resolve against the settings directory. Unknown keys are an error.

```yaml
tag_prefix: sndbx # optional, images are tagged <tag_prefix>:<name>
images_dir: ~/src/sndbx/images # optional, where build finds images, defaults to this repo's images/
docker_args: ["--security-opt=no-new-privileges", "--pids-limit=4096"] # optional, docker run arguments for every image
proxy: # optional, turns the proxy on
  image: ubuntu/squid@sha256:6a097f68bae708cedbabd6188d68c7e2e7a38cedd05a176e1cc0ba29e3bbe029
  allow: listed # optional, the hosts in allowed_hosts; all lets every host through
images:
  - name: claude
    tag: my-claude:latest # optional, defaults to <tag_prefix>:<name>
    user: ubuntu # optional, only needed for an image that runs as a numeric uid
    config_dir: ~/.claude_docker # optional, defaults to ~/.config/sndbx/<name>
    config_target: .claude # optional, relative to the user's home, defaults to .<name>
    docker_args: ["--memory=8g"] # optional, after the top-level ones
  - name: opencode
    config_target: .config/opencode
    docker_args: ["--add-host=host.docker.internal:host-gateway"]
  - name: codex
  - name: dsh
    ports: ["127.0.0.1:3080:3080"] # optional, always published; quote them
    proxy: false # optional, keeps the image on the open network when proxy is set
```

- **Hardening:** `no-new-privileges` keeps the agent from gaining root through `sudo` or `su`; `sndbx exec -u root` still works. `--pids-limit` contains fork bombs; raise it if builds fail with "Resource temporarily unavailable".
- **`docker_args` order:** top-level, the image's, then `run -- ...`. The last value of a flag like `--memory` wins; flags like `-e` add up.

## Network proxy

With `proxy` set, sandboxes run on `sndbx-internal`, a docker network with no route out. They reach the internet only through `sndbx-proxy`, a squid container that passes ports 80 and 443. `run` sets `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY`; tools that ignore them can't connect. The docker host and local network stay reachable.

- `allow: listed` (default) allows only the hosts in `~/.config/sndbx/allowed_hosts`. An empty file blocks everything; a missing one is an error.
- `allow: all` allows every host but link-local ones like `169.254.169.254`, to watch traffic without limiting it.

`allowed_hosts` has one host per line; a leading `.` includes subdomains:

```
.anthropic.com
.openai.com
.github.com
.githubusercontent.com
.npmjs.org
.pypi.org
```

`run` starts the proxy and network when missing, and applies changes on the next `run`. Pin `proxy.image` by digest: `docker image inspect ubuntu/squid --format '{{index .RepoDigests 0}}'`.

`sndbx logs proxy` follows the requests, `--denied` only the denied ones, i.e. hosts to add. HTTPS shows only the host. If the proxy doesn't start, see `docker logs sndbx-proxy`; its `squid.conf` is next to `sndbx.log`.

**Limits:**

- An image with `ports` or its own `--network` needs `proxy: false`; docker allows neither on an internal network.
- An agent can still send data out through an allowed host, like a GitHub gist.
- All proxied sandboxes on a docker host share one proxy, with the settings of the latest `run`.

**Remove** the proxy with `docker rm -f -v sndbx-proxy && docker network rm sndbx-internal`.

## Dashboard

`sndbx top` shows the sandboxes, the proxy's requests and the images, and acts on them. It runs `sndbx-top` from sndbx's directory or `PATH`, see [Install](#install).

```
 sndbx  [1 Dashboard]  2 Images                        docker ✓ │ proxy ✓ allows listed
┌ Sandboxes ───────────────────────────────────────────────────────────────────────────┐
│ NAME        IMAGE    STATE         UP     CPU     MEM      PIDS  NET ↑  NET ↓  EGRESS│
│▶api-1       codex    running      14m    0.3%    201M   38/4096  12.3K   3.1M  proxy │
│ dsh         dsh      running       3d    1.2%    340M        61   230K   5.6M  open  │
│ my-project  claude   running    2h13m    4.1%    612M  143/4096   1.2M    48M  proxy │
│ old-thing   claude   stopped                                                   proxy │
├ api-1 · 172.18.0.4 ──────────────────────────────────────────────────────────────────┤
│ mounts   ~/src/api → /workspace/api (workdir)                                        │
│ config   ~/.config/sndbx/codex → /home/ubuntu/.codex                                 │
│ hosts    sentry.io 2 · github.com 1                                                  │
└──────────────────────────────────────────────────────────────────────────────────────┘
┌ Proxy log · all · 4 denied ──────────────────────────────────────────────────────────┐
│11:41:03 172.18.0.4   200 nexus.example.com:443                                       │
│12:39:03 api-1        200 github.com:443                                              │
│12:39:10 my-project   200 api.anthropic.com:443                                       │
│12:39:17 api-1        403 sentry.io:443                                               │
│12:39:21 my-project   403 registry.yarnpkg.com:443                                    │
│12:39:27 api-1        403 sentry.io:443                                               │
│12:39:30 172.18.0.9   403 telemetry.x.dev                                             │
│                                                                                      │
│                                                                                      │
└──────────────────────────────────────────────────────────────────────────────────────┘
 tab focus  enter shell  s stop  d remove  f filter  z zoom  q quit
```

- **Dashboard** (`1`): the sandboxes, the selected one's mounts, ports and the hosts it asked for, the proxy's log with each request's sandbox and how many it denied. `NET ↑` and `NET ↓` are the bytes a sandbox sent and received on the network since it started, `EGRESS` whether it goes through the proxy or the open network. The log takes half the screen at most, the details what the sandboxes leave; `z` on the sandboxes shows all their hosts.
- **Images** (`2`): the configured images, `base`, and those in `images_dir` that aren't configured yet, with their size, age and sandboxes. ⚠ marks an image older than `base`. Below the table are the selected image's settings, with its `docker_args` merged as `run` passes them.

| Key                | Does                                                                                         |
| ------------------ | -------------------------------------------------------------------------------------------- |
| `tab`, `shift-tab` | moves between the panes                                                                      |
| `↑` `↓` `g` `G`    | selects, or scrolls the log; `G` follows new requests again                                  |
| `enter`            | opens a shell in the sandbox; in Images, shows the image's sandboxes, `esc` all again        |
| `r`                | resumes a stopped sandbox                                                                    |
| `s`, `d`           | stops or removes the sandbox, after asking; in Images, removes the image like `sndbx rm`     |
| `b`, `B`           | in Images, builds the image, or rebuilds base first, like `sndbx build`                      |
| `f`, `z`           | filters the log: all, denied, the selected sandbox's; zooms the pane                         |
| wheel, click       | scrolls the pane under the pointer; focuses a pane and selects the row, or switches the view |
| `q`                | quits                                                                                        |

- It reads docker without logging to `sndbx.log`; what the keys run is logged.
- Squid logs an HTTPS request when its tunnel closes, so a long one shows up late in the log and the traffic. Denied requests show right away.
- The proxy logs a sandbox by its address, which docker gives the next container once it's gone. A request counts as a sandbox's if it came from its address after it started; earlier ones show the address. The hosts' totals are those of the last 10,000 requests.
- Colors are the terminal's own 16, so they follow its theme, but for the selected row's dark gray background: each sandbox has one in the table and the log, CPU, memory and pids turn yellow, then red, near their limits, and `open` egress and `allows all` are yellow. `NO_COLOR=1` turns them off.
- It takes the mouse, like htop or vim with `mouse=a`: to select text, hold Shift while dragging, or Option in iTerm2.
- It needs a terminal of 80×20 at least. `sndbx top --once` prints a frame as text instead.

## Build

```sh
sndbx build claude             # build sndbx:claude, and sndbx:base first if it doesn't exist
sndbx build claude codex       # build both, in order
sndbx build -b claude          # rebuild sndbx:base first
sndbx build -b                 # rebuild sndbx:base only
```

`sndbx build` builds from `images/<name>/` without the docker cache, and adds new images to the settings with the keys from their `sndbx.yml`, so `sndbx run <name>` works right away. A downloaded binary needs `images_dir`, see [Install](#install).

### Add an image

Create `images/<name>/Dockerfile` starting with:

```dockerfile
ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base
```

If it needs more than a name in the settings, put those keys in `images/<name>/sndbx.yml`:

```yaml
ports: ["127.0.0.1:3080:3080"]
proxy: false
```

`build` copies them into the entry it adds; later changes to the file don't update an existing entry.

## Linux file permissions

Bind mounts keep the host's file owners, so on Linux `sndbx build` builds `sndbx:base` with your uid and gid, and the images on it can write to the mounts. It keeps uid 1000 when built as root, and on rootless or userns-remapped docker, where the mounts aren't writable and sndbx doesn't work around it.

For an image not built on base, grant its uid access with ACLs, here 1000:

```sh
setfacl -m u:1000:rwx,d:u:1000:rwx ~/.config/sndbx/claude
setfacl -R -m u:1000:rwX,d:u:1000:rwX ~/src/my-project
```

## Images

| Image      | Config in container             | Notes                                   |
| ---------- | ------------------------------- | --------------------------------------- |
| `claude`   | `/home/ubuntu/.claude`          |                                         |
| `opencode` | `/home/ubuntu/.config/opencode` | can use Ollama on the host, see below   |
| `codex`    | `/home/ubuntu/.codex`           | runs without its own sandbox, see below |
| `cursor`   | `/home/ubuntu/.cursor`          | runs without its own sandbox, see below |
| `dsh`      | `/home/ubuntu/.dsh`             | web UI on port 3080, see below          |

All are built on `sndbx:base`: Ubuntu with node, pnpm, python, ruby, rust, uv and postgres via mise, and Playwright with headless Chromium.

**opencode:** `images/opencode/opencode/opencode.json` is an example config for Ollama on the host; copy it into opencode's config directory. On Linux, Ollama must listen beyond loopback, e.g. `OLLAMA_HOST=0.0.0.0 ollama serve`.

**codex:** its own sandbox needs user namespaces, which docker blocks, so it runs with `--sandbox danger-full-access` and the container is the sandbox.

**cursor:** runs with `--sandbox disabled`, like codex. The image links `~/.config/cursor`, where cursor keeps its login, into `~/.cursor`, so one config directory holds both.

**dsh:** published on `127.0.0.1:3080` only. Keep it that way: a plain `-p 3080:3080` lets anyone on your network control the agent.

## Remove

```sh
sndbx rm claude codex
```

This removes the images and comments out their settings entries; building again adds a fresh one. Config directories, with logins and history, stay.

It checks all images before removing any, and refuses:

- `base`
- an image neither in the settings nor built
- an image a container uses; `docker rm` those first
- a tag another entry also runs, unless both are removed

## Run without sndbx

```sh
docker run --rm -it \
  --security-opt=no-new-privileges --pids-limit=4096 \
  -v ".:/workspace" \
  -v "$HOME/.claude:/home/ubuntu/.claude" \
  sndbx:claude
```

For other images, mount the config directory from the [table](#images) and add their flags from the [settings example](#configure).

## Test

```sh
bundle install                             # once
bundle exec rake                           # rustfmt, clippy, unit tests of both binaries, then every test, with rubocop alongside
bundle exec ruby test/completion_test.rb   # one file, against the last build
```

The tests run `target/debug/sndbx` (or `$SNDBX_BIN`) with a temporary `HOME` and a fake `docker`, so they need neither Docker nor existing settings.

## License

[MIT](LICENSE)
