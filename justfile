port := '8080'
backup_interval := '20'
backup_path := './backup'

build:
    cargo build --release

build-image:
    docker build -t erfansafari/efis .
    
release: build-image
    docker push erfansafari/efis:latest

run: build
    port={{port}} BACKUP_PATH={{backup_path}} BACKUP_INTERVAL={{backup_interval}} ./target/release/efis

run-docker:
    docker run --env "PORT={{port}}" --env "BACKUP_PATH={{backup_path}}" --env "BACKUP_INTERVAL={{backup_interval}}" -p 8080:8080 erfansafari/efis
