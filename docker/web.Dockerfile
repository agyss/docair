#***
# The docair-web container: the built Angular app, served by nginx.
#
# Node builds the app, then the static files go into an unprivileged nginx
# image that listens on 8080. The browser downloads them and runs the app.
#
# @id      container.docair-web
# @kind    container
# @part-of docker
# @reads   images (docair-web)
#***
FROM node:24-alpine AS build
WORKDIR /src
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npx ng build --configuration production

FROM nginxinc/nginx-unprivileged:1.30-alpine
# The image's entrypoint fills in DOCAIR_SERVER and writes the result to conf.d.
COPY docker/nginx.conf /etc/nginx/templates/default.conf.template
ENV DOCAIR_SERVER=docair-server:8080
COPY --from=build /src/dist/docair-web/browser /usr/share/nginx/html
EXPOSE 8080
