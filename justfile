port := '9393'
persist_interval := '20'
persist_path := '/home/erfan/.local/efis'

build:
    cargo build --release

buildi:
    docker build -t erfansafari/efis .

release: buildi
    docker push erfansafari/efis:latest

run: build
    PORT={{port}} PERSIST_PATH={{persist_path}} PERSIST_INTERVAL={{persist_interval}} ./target/release/efis

run1: build
    ID=n1 \
    PORT=3333 \
    PERSIST_PATH={{persist_path}}1 \
    PERSIST_INTERVAL={{persist_interval}} \
    PEERS=localhost:3334,localhost:3335 \
    ./target/release/efis

run2: build
    ID=n2 \
    PORT=3334 \
    PERSIST_PATH={{persist_path}}2 \
    PERSIST_INTERVAL={{persist_interval}} \
    PEERS=localhost:3333,localhost:3335 \
    ./target/release/efis

run3: build
    ID=n3 \
    PORT=3335 \
    PERSIST_PATH={{persist_path}}3 \
    PERSIST_INTERVAL={{persist_interval}} \
    PEERS=localhost:3333,localhost:3334 \
    ./target/release/efis

run4: build
    ID=n4 \
    PORT=3336 \
    PERSIST_PATH={{persist_path}}4 \
    PERSIST_INTERVAL={{persist_interval}} \
    PEERS=localhost:3333,localhost:3334,localhost:3335 \
    ./target/release/efis

runi:
    docker run \
        --env "PORT={{port}}" \
        --env "PERSIST_PATH={{persist_path}}" \
        --env "PERSIST_INTERVAL={{persist_interval}}" \
    -p 8080:8080 erfansafari/efis
