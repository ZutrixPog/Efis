# Efis - A Simple Key-Value Store (and PubSub)

Efis is a lightweight and efficient key-value store written in Rust. It provides a simple and user-friendly interface to store, retrieve, and manage key-value pairs. Efis offers features such as automatic backups, pub/sub messaging, and easy deployment using Docker. **Efis is not intended to be used in production**. Study the code, improve it and use it as an opportunity to practice. This README provides an overview of Efis, its features, and instructions on how to build and use it.

## Features

- **Simple Key-Value Store:** Efis allows you to store key-value pairs where the key is a string, and the value can be a string, list, set, or sorted set.

- **Automatic Backups:** Efis automatically backs up its data to a specified directory at a configurable interval allowing you to have some sort of **Persistance**.

- **Pub/Sub Messaging:** Efis supports pub/sub messaging, allowing multiple clients to subscribe to specific keys and receive updates when the value of those keys changes.

- **Docker Support:** Efis can be easily built and deployed using Docker, making it portable and convenient to run on various systems.

## Commands

Efis supports the following commands:

- `SET`: Set a key-value pair in the store.
- `GET`: Retrieve the value for a given key.
- `DELETE`: Delete a key-value pair.
- `INCREMENT`: Increment the value of a numeric key by 1.
- `DECREMENT`: Decrement the value of a numeric key by 1.
- `EXPIRE`: Set a time-to-live (TTL) for a key. The key will be automatically deleted after the specified time.
- `TTL`: Get the remaining time-to-live for a key.
- `LPUSH`: Insert elements at the beginning of a list.
- `RPUSH`: Insert elements at the end of a list.
- `LPOP`: Remove and return the first element of a list.
- `RPOP`: Remove and return the last element of a list.
- `SADD`: Add elements to a set.
- `SMEMBERS`: Get all elements of a set.
- `ZADD`: Add elements to a sorted set with a numeric score.
- `ZRANGE`: Get a range of elements from a sorted set.

It would be a great exercise reading the code and trying to find out more about commands and the underlying protocol. :D

## How to Build

To build Efis, you'll need [Rust](https://www.rust-lang.org/tools/install) installed on your system.

1. Clone the Efis repository from GitHub:
```
git clone https://github.com/your_username/efis.git
cd efis
```
2. Build Efis using just:
```
just build
```
or using cargo:
```
cargo build --release
```

## How to Use

To run Efis, you can use the following command:
```
just run
```
you can modify the justfile to include your own prefrences.

- modify `port` to the port number you want Efis to listen on.
- modify `backup_path` to the path where Efis will store its backups.
- modify `backup_interval` to your desired backup interval in seconds.

## How to build with Docker

To run Efis using Docker, follow these step:

1. Build the Docker image:
```
just build-image
```
2. Run the Efis container:
```
just run-docker
```

## How to run with docker
Alternatively, you can pull and run the image as follows:
```
docker pull erfansafari/efis
docker run --env PORT=your-port --env BACKUP_PATH=your-backup_path --env BACKUP_INTERVAL=your-backup_interval erfansafari/efis
```


## Contributions

Contributions to Efis are welcome! This is an educational project and there are so many features that can be added.
here's a few issues you can work on:
- a client app (probably the most important and helpful)
- more command support
- bug fixes
- performance boost
- use your imagination

Feel free to submit pull requests to improve the project and your skills.

## License

Efis is open-source and licensed under the [MIT License](LICENSE). You are free to use, modify, and distribute the software according to the terms of the license.

---
