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

###################################
#### Final lightweight image ######
###################################
FROM debian:trixie-slim AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/i2g-operator /usr/local/bin

ENTRYPOINT ["/usr/local/bin/i2g-operator"]
