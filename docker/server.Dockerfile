#***
# The docair-server container: the server binary on a distroless base.
#
# Built in two stages: the Rust toolchain compiles a release binary, then only
# that binary is copied onto a small base image without a shell or package
# manager. It runs as an unprivileged user and listens on 8080.
#
# @id      container.docair-server
# @kind    container
# @part-of k8s.deploy.docair-server
# @reads   images (docair-server:dev)
# @flow    provision 10 "containerd starts both containers"
#***
FROM rust:1.99-slim-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p docair-server

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /src/target/release/docair-server /usr/local/bin/docair-server
USER nonroot
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/docair-server"]
