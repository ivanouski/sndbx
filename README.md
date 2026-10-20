# sndbx

Docker images for running coding agents in a sandbox, plus `sndbx`, a small wrapper around `docker run`.

| Image      | Directory          | Config on host | Config in container             |
| ---------- | ------------------ | -------------- | ------------------------------- |
| `claude`   | `images/claude/`   | `~/.claude`    | `/home/ubuntu/.claude`          |
| `opencode` | `images/opencode/` | `~/.opencode`  | `/home/ubuntu/.config/opencode` |
| `codex`    | `images/codex/`    | `~/.codex`     | `/home/ubuntu/.codex`           |
| `dsh`      | `images/dsh/`      | `~/.dsh`       | `/home/ubuntu/.dsh` (port 3080) |

The host paths are the ones the `docker run` examples below mount. `sndbx` mounts `~/.config/sndbx/<name>` unless an image sets `config_dir`.

`images/opencode/opencode/config.json` is an example opencode config that uses Ollama running on the host. The image doesn't include it, so copy it into opencode's config directory on the host to use it. The examples below pass `--add-host=host.docker.internal:host-gateway` so the container can reach the host on Linux too, where Ollama must also listen beyond loopback, e.g. `OLLAMA_HOST=0.0.0.0 ollama serve`.

All of them are built on `<tag_prefix>:base` (`sndbx:base` by default), the base image in `images/base/` (Ubuntu with node, python, ruby, rust, uv and postgres via mise, and Playwright with headless Chromium).

dsh refuses to listen on a wildcard address such as `0.0.0.0` because it "would expose remote code execution to the network", so the `dsh` image binds the container's own address instead, which a published port reaches. Publish the port on `127.0.0.1` only, as the examples below do. A plain `-p 3080:3080` lets anyone on your network control the agent.

## Build

`sndbx build IMAGE...` builds each `images/IMAGE/` in turn, always with `--no-cache`. Every image is tagged `<tag_prefix>:<directory name>`, e.g. `sndbx:base`, `sndbx:claude`. The prefix is `sndbx` unless `tag_prefix` is set in the [settings](#configure).

```sh
sndbx build claude                    # build/rebuild sndbx:claude, and sndbx:base first if it doesn't exist yet
sndbx build claude codex              # build/rebuild sndbx:claude, then sndbx:codex
sndbx build --rebuild-base claude dsh # build/rebuild sndbx:base, then sndbx:claude and sndbx:dsh
sndbx build --rebuild-base            # build/rebuild sndbx:base only
sndbx list buildable                  # print the images build accepts
```

`sndbx build claude` won't rebuild an existing `sndbx:base`. To pick up base changes, add `--rebuild-base` (`-b`).

`sndbx` finds `images/` in the repo it was built from, wherever the binary is. If the repo moves, or the binary was built in another checkout or container, set `images_dir` in the [settings](#configure).

To add an image, create `images/<name>/Dockerfile` starting with:

```dockerfile
ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base
```

## Run with sndbx

`sndbx` wraps the `docker run` commands below with less typing. It:

- mounts each `-v SRC[:DST[:OPTS]]` like `docker run -v`, at `/workspace/<basename>` when there's no `DST`
- mounts the current directory at `/workspace/<basename>` unless a volume is mounted in `/workspace`
- mounts the image's config directory at `/home/<user>/<config_target>`, where `<user>` is the one the image runs as, or `/root/<config_target>` for root
- publishes the image's `ports` plus each `-p PORT`
- passes the top-level `docker_args`, then the image's, then everything after `--` to `docker run`
- labels the container `sndbx.image=<name>`, which `sndbx exec` and `sndbx resume` find it by
- logs each `docker run`, `exec`, `start` and `build` it runs to `~/.local/state/sndbx/sndbx.log` (or under `$XDG_STATE_HOME`), which `sndbx logs` follows

### Install

`sndbx` is written in Rust. Build and install it into `~/.cargo/bin` from the repo with [mise](https://mise.jdx.dev)'s Rust or any other recent toolchain, on the machine that runs it:

```sh
cargo install --path .
```

Optionally, install the bash completion, it needs bash 4.0 or later. With bash-completion it loads on demand from:

```sh
cp completions/_sndbx ~/.local/share/bash-completion/completions/sndbx
```

Without bash-completion, source `completions/_sndbx` in `~/.bashrc` instead.

The completion completes arguments after `--` with docker's own completion, `docker __complete run`, and falls back to file names when docker doesn't have it.

### Configure

Images are listed in `~/.config/sndbx/settings.yml`, or `$XDG_CONFIG_HOME/sndbx/settings.yml` when `XDG_CONFIG_HOME` is set, which also moves the default `config_dir`s and where relative paths resolve. The first run creates an example to edit. An unknown key is an error, so a misspelled one like `docker_arg` doesn't go unnoticed.

```yaml
tag_prefix: sndbx # optional, images are tagged <tag_prefix>:<name>
images_dir: ~/src/sndbx/images # optional, where build finds images, defaults to the repo's images/; relative to ~/.config/sndbx
docker_args: ["--security-opt=no-new-privileges", "--pids-limit=4096"] # optional, docker run arguments for every image
images:
  - name: claude
    tag: my-claude:latest # optional, defaults to <tag_prefix>:<name>
    user: ubuntu # optional, whose home config_target is in, defaults to the user the image runs as
    # config_dir: ~/.claude_docker # optional, relative paths resolve against ~/.config/sndbx, defaults to ~/.config/sndbx/<name>
    config_target: .claude # optional, relative to the user's home, defaults to .<name>
    docker_args: ["--memory=8g"] # optional, extra docker run arguments, after the top-level ones
  - name: opencode
    config_target: .config/opencode
    docker_args: ["--add-host=host.docker.internal:host-gateway"] # optional, lets the container reach Ollama on the host on Linux
  - name: codex
  - name: dsh
    ports: ["127.0.0.1:3080:3080"] # optional, always published; quote them
```

`--security-opt=no-new-privileges` keeps processes in the sandbox from gaining privileges, through setuid binaries like `su` or file capabilities, so the agent stays the unprivileged user. `--pids-limit=4096` caps the processes and threads in the sandbox, so a fork bomb or a runaway script fails there instead of filling the host's process table. Threads count too, and Node, cargo and Chromium use many, so raise the limit if builds fail with "Resource temporarily unavailable"; `docker stats` shows the count. The example settings sndbx creates set both.

sndbx reads the user from the image, `USER` in its Dockerfile, so `user` is only needed to override it, or when the image runs as a numeric uid, whose name sndbx can't tell.

The top-level `docker_args` come first, so an image overrides a flag that takes one value, like `--pids-limit=8192` or `--memory`, with its own: docker takes the last. Flags that add up, like `--security-opt`, `--cap-add` or `-e`, add to the top-level ones.

### Use

```sh
sndbx run claude                                      # mount the current directory
sndbx run claude -v ../api -v ../web                  # mount several, start in the last one
sndbx run claude -v ../api -v ../web -w api           # mount several, start in api
sndbx run claude -v ./garmin/sdk:/opt/garmin/sdk:ro   # also mount a read-only SDK, still start in the current directory
sndbx run claude -p 3000 -p 8080:80                   # publish ports, 3000 means 3000:3000
sndbx run claude -- --memory=8g -e FOO=bar            # pass extra arguments to docker run
sndbx run claude -k                                   # keep the container after it exits, for sndbx resume
sndbx exec claude gifted_mcnulty bash                 # run a command in a running claude sandbox, like docker exec -it
sndbx exec -u root claude gifted_mcnulty bash         # as root
sndbx resume claude sleepy_babbage                    # restart a stopped claude sandbox, like docker start -ai
sndbx list configured                                 # print the image names in the settings
sndbx list buildable                                  # print the image names build accepts
sndbx list running claude                             # print the running claude sandboxes
sndbx list stopped claude                             # print the stopped claude sandboxes
sndbx logs                                            # follow the log of docker commands, like tail -f
sndbx logs -n 50                                      # start with the last 50 lines
sndbx --version
```

`sndbx exec` finds sandboxes by their `sndbx.image` label, so it doesn't see sandboxes started before sndbx 0.15.0. It passes `-t` to `docker exec` only when stdin is a terminal, so it works in pipes and scripts.

## Run with docker

Each command mounts the current directory at `/workspace` and the image's config directory from the table above.

```sh
docker run --rm -it \
  -v ".:/workspace" \
  -v "$HOME/.claude:/home/ubuntu/.claude" \
  sndbx:claude

docker run --rm -it \
  --add-host=host.docker.internal:host-gateway \
  -v ".:/workspace" \
  -v "$HOME/.opencode:/home/ubuntu/.config/opencode" \
  sndbx:opencode

docker run --rm -it \
  -v ".:/workspace" \
  -v "$HOME/.codex:/home/ubuntu/.codex" \
  sndbx:codex

docker run --rm -it \
  -p 127.0.0.1:3080:3080 \
  -v ".:/workspace" \
  -v "$HOME/.dsh:/home/ubuntu/.dsh" \
  sndbx:dsh
```

## Test

```sh
bundle install                                # once, installs minitest, rake and rubocop
bundle exec rake                              # rustfmt, clippy, unit tests, rubocop, then every test
bundle exec rake test                         # build, then every test
bundle exec ruby test/completion_test.rb      # one file, against the last build
```

The tests are in Ruby, a file per command, plus `settings_test.rb` and `completion_test.rb`. They run `target/debug/sndbx`, or the binary `SNDBX_BIN` names, with a temporary `HOME` and a fake `docker` that records its arguments, so they need neither Docker nor existing settings. The completion tests call `_sndbx` in bash 4 or newer, like the completions themselves.

## License

[MIT](LICENSE)
