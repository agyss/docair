#***
# The docair-web container: the built Angular app, served by nginx.
#
# Node builds the app, then the static files go into an unprivileged nginx
# image that listens on 8080. The browser downloads them and runs the app.
#
# @id      container.docair-web
# @kind    container
# @part-of k8s.deploy.docair-web
# @reads   images (docair-web:dev)
# @flow    provision 10 "containerd starts both containers"
#***
FROM node:24-alpine AS build
WORKDIR /src
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npx ng build --configuration production

FROM nginxinc/nginx-unprivileged:1.30-alpine
COPY docker/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /src/dist/docair-web/browser /usr/share/nginx/html
EXPOSE 8080
