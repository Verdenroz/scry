FROM rust:1.98-alpine AS build
RUN apk add --no-cache build-base
WORKDIR /src
COPY . .
RUN cargo build --release -p scry

FROM alpine:3.22
RUN apk add --no-cache ca-certificates
COPY --from=build /src/target/release/scry /usr/local/bin/scry
EXPOSE 7345
ENTRYPOINT ["scry", "serve"]
