FROM ubuntu:26.10

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=C.UTF-8 LC_ALL=C.UTF-8

# The agent can't apt install, so the compiler stays for what mise, npm or pip build from source
RUN apt update && apt upgrade -y && apt install -y --no-install-recommends \
  ca-certificates tzdata tini curl git less \
  jq ripgrep fd-find unzip \
  build-essential pkg-config \
  && rm -rf /var/lib/apt/lists/* \
  && ln -s /usr/bin/fdfind /usr/local/bin/fd

ENTRYPOINT ["/usr/bin/tini", "--"]

ARG USER=ubuntu
# sndbx passes the host user's on Linux, so the agent can write to the mounts
ARG UID=1000
ARG GID=1000

# Before mise, so it's owned by the user too; -o allows ids the image already has.
# usermod moves the user's record in lastlog and faillog to an offset of uid × record size, which a layer stores in
# full, so a large uid would make it huge
RUN rm -f /var/log/lastlog /var/log/faillog \
  && groupmod -o -g "${GID}" "${USER}" \
  && usermod -o -u "${UID}" "${USER}" \
  && chown -R "${UID}:${GID}" "/home/${USER}"

USER ${USER}
ENV HOME=/home/${USER}

RUN curl -fsSL https://mise.run | bash

ENV PATH=${HOME}/.local/bin:${HOME}/.local/share/mise/shims:$PATH

WORKDIR ${HOME}
