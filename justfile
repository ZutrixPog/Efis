port := '8080'
backup_interval := '120'
backup_path := './backup'

build:
    cargo build --release

build-image:
    docker build -t efis .
    docker tag efis erfansafari/efis:latest

release:
    docker push erfansafari/efis:latest

run: build
    port={{port}} BACKUP_PATH={{backup_path}} BACKUP_INTERVAL={{backup_interval}} ./target/release/efis

run-docker:
    docker run --env PORT={{port}} --env BACKUP_PATH={{backup_path}} --env BACKUP_INTERVAL={{backup_interval}} efis
