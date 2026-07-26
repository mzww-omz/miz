FROM node:24-bookworm-slim AS build
RUN corepack enable
WORKDIR /src
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/web/package.json apps/web/package.json
COPY packages packages
RUN pnpm install --frozen-lockfile
COPY apps/web apps/web
RUN pnpm --filter @miz/web build

FROM node:24-bookworm-slim
WORKDIR /app
COPY --from=build /src/apps/web/build ./build
ENV HOST=0.0.0.0 PORT=3000
EXPOSE 3000
CMD ["node", "build"]
