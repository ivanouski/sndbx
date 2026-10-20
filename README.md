# sndbx

Docker images for running coding agents in a sandbox, plus `sndbx`, a small wrapper around `docker run`.

## Images

| Image      | Config in container             | Notes                                     |
| ---------- | ------------------------------- | ----------------------------------------- |
| `claude`   | `/home/ubuntu/.claude`          |                                           |
| `opencode` | `/home/ubuntu/.config/opencode` | can use Ollama on the host, see below     |
| `codex`    | `/home/ubuntu/.codex`           |                                           |
| `dsh`      | `/home/ubuntu/.dsh`             | web UI on port 3080, publish on 127.0.0.1 |

All are built on `sndbx:base` (`images/base/`): Ubuntu with node, python, ruby, rust, uv and postgres via mise, and Playwright with headless Chromium.

**opencode with Ollama:** `images/opencode/opencode/config.json` is an example config for Ollama on the host; copy it into opencode's config directory. The container reaches the host with `--add-host=host.docker.internal:host-gateway`, and on Linux Ollama must listen beyond loopback, e.g. `OLLAMA_HOST=0.0.0.0 ollama serve`.

**dsh** refuses to listen on a wildcard address, so the image binds the container's own address, which a published port reaches. Publish it on `127.0.0.1` only: a plain `-p 3080:3080` lets anyone on your network control the agent.

## Install

Build and install `sndbx` into `~/.cargo/bin` with a recent Rust toolchain, e.g. [mise](https://mise.jdx.dev)'s:

```sh
cargo install --path .
```

Optionally, install the bash completion (bash 4.0+). It completes arguments after `--` with docker's own completion.

```sh
cp completions/_sndbx ~/.local/share/bash-completion/completions/sndbx   # with bash-completion
# or source completions/_sndbx in ~/.bashrc
```

## Configure

Settings live in `~/.config/sndbx/settings.yml` (`$XDG_CONFIG_HOME/sndbx/` if set). `run` creates an example when there's none, and `build` creates it with the image it built. Relative paths resolve against the settings directory. Unknown keys are an error, so typos don't go unnoticed.

```yaml
tag_prefix: sndbx # optional, images are tagged <tag_prefix>:<name>
images_dir: ~/src/sndbx/images # optional, where build finds images, defaults to this repo's images/
docker_args: ["--security-opt=no-new-privileges", "--pids-limit=4096"] # optional, docker run arguments for every image
images:
  - name: claude
    tag: my-claude:latest # optional, defaults to <tag_prefix>:<name>
    user: ubuntu # optional, defaults to the user the image runs as
    config_dir: ~/.claude_docker # optional, defaults to ~/.config/sndbx/<name>
    config_target: .claude # optional, relative to the user's home, defaults to .<name>
    docker_args: ["--memory=8g"] # optional, after the top-level ones
  - name: opencode
    config_target: .config/opencode
    docker_args: ["--add-host=host.docker.internal:host-gateway"]
  - name: codex
  - name: dsh
    ports: ["127.0.0.1:3080:3080"] # optional, always published; quote them
```

- **Hardening:** `no-new-privileges` stops processes from gaining privileges through setuid binaries like `su` or `sudo`, so the agent stays unprivileged; `sndbx exec -u root` still works. `--pids-limit` caps processes and threads, so a fork bomb fails in the sandbox, not on the host. Node, cargo and Chromium use many threads: raise it if builds fail with "Resource temporarily unavailable".
- **`docker_args` order:** top-level first, then the image's, then `run -- ...`. For a flag that takes one value, like `--memory`, the last one wins; flags like `-e` or `--cap-add` add up.
- **`user`** is only needed when the image runs as a numeric uid, whose home sndbx can't tell.
- **`images_dir`** defaults to the repo the binary was built from. Set it if the repo moved or the binary was built elsewhere.

## Use

```sh
sndbx run claude                                      # mount the current directory
sndbx run claude -v ../api -v ../web                  # mount several, start in the last one
sndbx run claude -v ../api -v ../web -w api           # start in api
sndbx run claude -v ./sdk:/opt/sdk:ro                 # mount a read-only SDK, start in the current directory
sndbx run claude -p 3000 -p 8080:80                   # publish ports, 3000 means 3000:3000
sndbx run claude -k                                   # keep the container after it exits, for resume
sndbx run claude -- --memory=8g -e FOO=bar            # pass extra arguments to docker run
sndbx exec claude gifted_mcnulty bash                 # run a command in a running sandbox
sndbx exec -u root claude gifted_mcnulty bash         # as root
sndbx resume claude sleepy_babbage                    # restart a stopped sandbox
sndbx list configured                                 # image names in the settings
sndbx list buildable                                  # image names build accepts
sndbx list running claude                             # claude's running sandboxes
sndbx list stopped claude                             # claude's stopped sandboxes
sndbx logs                                            # follow the log of the docker commands sndbx ran
sndbx logs -n 50                                      # start with the last 50 lines
```

`sndbx run`:

- mounts each `-v SRC[:DST[:OPTS]]` like `docker run -v`, at `/workspace/<basename>` when there's no `DST`
- mounts the current directory at `/workspace/<basename>` unless a volume is mounted in `/workspace`
- mounts the image's config directory at `/home/<user>/<config_target>`, where `<user>` is the one the image runs as, or `/root/<config_target>` for root
- publishes the image's `ports` plus each `-p PORT`
- passes the top-level `docker_args`, then the image's, then everything after `--` to `docker run`
- labels the container `sndbx.image=<name>`, which `exec`, `resume` and `list` find it by

Every docker command sndbx runs is logged to `~/.local/state/sndbx/sndbx.log` (`$XDG_STATE_HOME/sndbx/` if set), which `sndbx logs` follows.

## Build

```sh
sndbx build claude             # build sndbx:claude, and sndbx:base first if it doesn't exist
sndbx build claude codex       # build both, in order
sndbx build -b claude          # rebuild sndbx:base first
sndbx build -b                 # rebuild sndbx:base only
```

`sndbx build`:

- builds each image from `images/<name>/`, without the docker cache
- builds `sndbx:base` first if an image needs it and it doesn't exist, or with `-b`
- appends `- name: '<image>'` to the settings' `images` for an image that isn't there yet, so `sndbx run <image>` works right away; set `config_target` if the agent doesn't keep its config in `~/.<image>`
- prints the entry to add instead when it can't tell where to append

To add an image, create `images/<name>/Dockerfile` starting with:

```dockerfile
ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base
```

## Remove

```sh
sndbx rm claude codex
```

`sndbx rm`:

- removes the images, like `docker image rm`
- comments out their entries in the settings under `# Removed by sndbx rm on <date>:`, so the settings aren't lost; building again adds a fresh entry
- keeps their config directories, with the agents' logins and history

It checks every image before removing any, and refuses:

- `base`, which the other images are built on
- an image that's neither in the settings nor built
- an image that containers use, in any state; remove them with `docker rm` first
- a tag that another entry also runs, unless both are removed together

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
bundle exec rake                           # rustfmt, clippy, unit tests, rubocop, then every test
bundle exec ruby test/completion_test.rb   # one file, against the last build
```

The tests run `target/debug/sndbx` (or `$SNDBX_BIN`) with a temporary `HOME` and a fake `docker`, so they need neither Docker nor existing settings.

## License

[MIT](LICENSE)
