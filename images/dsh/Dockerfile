ARG TAG_PREFIX=sndbx
FROM ${TAG_PREFIX}:base

RUN git clone --depth 1 https://github.com/deepseek-ai/deepseek-harness ~/dsh

WORKDIR ${HOME}/dsh

RUN pnpm install \
  && pnpm run build

EXPOSE 3080/tcp

WORKDIR /workspace

CMD exec node ~/dsh/apps/cli/lib/bin.js web --no-open --host "$(hostname -i | cut -d' ' -f1)"
