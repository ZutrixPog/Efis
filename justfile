port := '8080'
backup_interval := '120'
backup_path := './backup'

build:
    cargo build --release

build-image:
    sudo docker build -t efis .
    docker tag efis erfansafari/efis:latest

release: build-image
    docker push erfansafari/efis:latest

run:
    PORT={{port}} BACKUP_PATH={{backup_path}} BACKUP_INTERVAL={{backup_interval}} cargo run

run-docker:
    docker run --env PORT={{port}} --env BACKUP_PATH={{backup_path}} --env BACKUP_INTERVAL={{backup_interval}} your_image_name
