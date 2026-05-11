# syntax=docker/dockerfile:1.7

FROM node:24-alpine AS deps
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci

FROM node:24-alpine AS build
WORKDIR /app
COPY --from=deps /app/node_modules ./node_modules
COPY . .
ENV NEXT_TELEMETRY_DISABLED=1
RUN npm run build

FROM node:24-alpine AS runtime
WORKDIR /app
ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1
ENV PORT=3000
ENV HOSTNAME=0.0.0.0
# Run as the unprivileged "node" user (uid/gid 1000) shipped with the
# base image instead of root. Any bind-mounted host directory (e.g.
# ./.app-state) must be writable by uid 1000 — set this on the host
# with `chown -R 1000:1000 ./.app-state`. Without USER, the runtime
# would write the bootstrap auth file as root and a compromised process
# could escalate within the container.
COPY --from=build --chown=node:node /app/public ./public
COPY --from=build --chown=node:node /app/.next/standalone ./
COPY --from=build --chown=node:node /app/.next/static ./.next/static
# Pre-create the state dir owned by node so an anonymous volume mount
# starts out writable. Bind mounts inherit host ownership and still need
# the chown on the host side.
RUN mkdir -p /app/state && chown -R node:node /app/state
USER node
EXPOSE 3000
CMD ["node", "server.js"]
