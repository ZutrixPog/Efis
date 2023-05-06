use std::{fs, fs::File};
use std::error::Error;
use rand::Rng;

fn save_data(path: String, data: &[u8]) -> Result<()> {
    let tmp = Path::new(format!("{}.{}", path, generate_tmp_name()));
    let fp = File::options()
                .read(true)
                .write(true)
                .create(true)
                .open(tmp)?;

    if let Err = fp.write_all(data) {
        fs::remove_file(path)?;
    }

    if let Err = fp.sync_all() {
        fs::remove_file(path)?;
    }

    fs::rename(tmp, path)
}

fn log_create(path: String) -> Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .append(true)
        .open(path)
}

fn log_append(fp: File, mut line: String) -> Result<()> {
    line.push('\n');
    let buf = line.as_bytes();
    fp.write_all(buf)?;

    fp.sync_all()
}

fn generate_tmp_name() -> String {
    let mut rng = rand::thread_rng();

    format!("tmp.{}", rng.gen::<i32>())
}