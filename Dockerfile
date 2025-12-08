##############################################
##### Base image for planner and builder #####
##############################################
FROM rust:1 AS chef 
# We only pay the installation cost once, 
# it will be cached from the second build onwards
RUN cargo install cargo-chef 
WORKDIR /app

####################################
##### Build planner for caches #####
####################################
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

####################################
##### Building an application ######
####################################
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin i2g-operator 

####################################
##### ORIGINAL i2g BINARY ##########
####################################
FROM golang:1.25.5-trixie AS i2g-bin

RUN go install github.com/kubernetes-sigs/ingress2gateway@v0.4.0

###################################
#### Final lightweight image ######
###################################
FROM debian:trixie-slim AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/i2g-operator /usr/local/bin
COPY --from=i2g-bin /go/bin/ingress2gateway /usr/local/bin/ingress2gateway

ENTRYPOINT ["/usr/local/bin/i2g-operator"]
