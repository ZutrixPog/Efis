port := '8080'
backup_interval := '20'
backup_path := '/home/erfan/.local/efis'

build:
    cargo build --release

build-image:
    docker build -t erfansafari/efis .

release: build-image
    docker push erfansafari/efis:latest

run: build
    port={{port}} BACKUP_PATH={{backup_path}} BACKUP_INTERVAL={{backup_interval}} ./target/release/efis

run1: build
    port=3333 BACKUP_PATH={{backup_path}}1 BACKUP_INTERVAL={{backup_interval}} peers=localhost:3334,localhost:3335 ./target/release/efis

run2: build
    port=3334 BACKUP_PATH={{backup_path}}2 BACKUP_INTERVAL={{backup_interval}} peers=localhost:3333,localhost:3335 ./target/release/efis

run3: build
    port=3335 BACKUP_PATH={{backup_path}}3 BACKUP_INTERVAL={{backup_interval}} peers=localhost:3333,localhost:3334 ./target/release/efis

run-docker:
    docker run --env "PORT={{port}}" --env "BACKUP_PATH={{backup_path}}" --env "BACKUP_INTERVAL={{backup_interval}}" -p 8080:8080 erfansafari/efis
