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

All of them are built on `<tag_prefix>:base` (`sndbx:base` by default), the base image in `images/base/` (Ubuntu with node, ruby and python via mise, postgres and Playwright dependencies).

dsh refuses to listen on a wildcard address such as `0.0.0.0` because it "would expose remote code execution to the network", so the `dsh` image binds the container's own address instead, which a published port reaches. Publish the port on `127.0.0.1` only, as the examples below do. A plain `-p 3080:3080` lets anyone on your network control the agent.

## Build

`sndbx build IMAGE` builds `images/IMAGE/`, always with `--no-cache`. Every image is tagged `<tag_prefix>:<directory name>`, e.g. `sndbx:base`, `sndbx:claude`. The prefix is `sndbx` unless `tag_prefix` is set in the [settings](#configure).

```sh
sndbx build claude    # build/rebuild sndbx:claude, and sndbx:base first if it doesn't exist yet
sndbx build base      # build/rebuild sndbx:base
sndbx build --all     # build/rebuild sndbx:base, then every other image
sndbx list buildable  # print the images build accepts
```

`sndbx build claude` won't rebuild an existing `sndbx:base`. To pick up base changes, run `sndbx build base` before it.

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
- mounts the image's config directory at `/home/<user>/<config_target>`
- publishes the image's `ports` plus each `-p PORT`
- passes the image's `docker_args` plus everything after `--` to `docker run`
- labels the container `sndbx.image=<name>`, which `sndbx exec` and `sndbx resume` find it by
- logs each `docker run`, `exec`, `start` and `build` it runs to `~/.local/state/sndbx/sndbx.log`, which `sndbx logs` follows

### Install

`sndbx` is written in Rust. Build and install it from the repo with [mise](https://mise.jdx.dev)'s Rust or any other recent toolchain, on the machine that runs it, and source the bash completions:

```sh
cargo install --path .                        # installs sndbx to ~/.cargo/bin
source /path/to/sndbx/completions/_sndbx
```

The completions complete arguments after `--` with docker's own completion, `docker __complete run`, and fall back to file names when docker doesn't have it.

### Configure

Images are listed in `~/.config/sndbx/settings.yml`. The first run creates an example to edit.

```yaml
tag_prefix: sndbx # optional, images are tagged <tag_prefix>:<name>
images_dir: ~/src/sndbx/images # optional, where build finds images, defaults to the repo's images/; relative to ~/.config/sndbx
images:
  - name: claude
    tag: my-claude:latest # optional, defaults to <tag_prefix>:<name>
    user: ubuntu
    # config_dir: ~/.claude_docker # optional, relative paths resolve against ~/.config/sndbx, defaults to ~/.config/sndbx/<name>
    config_target: .claude # optional, relative to the user's home, defaults to .<name>
    docker_args: ["--memory=8g"] # optional, extra docker run arguments
  - name: opencode
    user: ubuntu
    config_target: .config/opencode
    docker_args: ["--add-host=host.docker.internal:host-gateway"] # optional, lets the container reach Ollama on the host on Linux
  - name: codex
    user: ubuntu
  - name: dsh
    user: ubuntu
    ports: ["127.0.0.1:3080:3080"] # optional, always published; quote them
```

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
rake                              # rustfmt, clippy, unit tests, rubocop, then every test
rake test                         # build, then every test
ruby test/completion_test.rb      # one file, against the last build
```

The tests are in Ruby. They run `target/debug/sndbx`, or the binary `SNDBX_BIN` names, with a temporary `HOME` and a fake `docker` that records its arguments, so they need neither Docker nor existing settings. The completion tests call `_sndbx` in bash 4 or newer, like the completions themselves.

## License

[MIT](LICENSE)
