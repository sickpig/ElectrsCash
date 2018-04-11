extern crate bitcoin;
extern crate byteorder;
extern crate crypto;
extern crate rocksdb;
extern crate time;

use self::rocksdb::{DBCompactionStyle, DBCompressionType, WriteBatch, WriteOptions, DB};
use bitcoin::blockdata::block::BlockHeader;
use bitcoin::network::serialize::deserialize;
use std::char::from_digit;
use std::collections::HashMap;
use self::time::{Duration, PreciseTime};

use Bytes;

pub struct Store {
    db: DB,
    rows: Vec<Row>,
    start: PreciseTime,
}

pub struct Row {
    pub key: Bytes,
    pub value: Bytes,
}

pub struct StoreOptions {
    pub auto_compact: bool,
}

fn revhex(data: &[u8]) -> String {
    let mut ret = String::with_capacity(data.len() * 2);
    for item in data.iter().rev() {
        ret.push(from_digit((*item / 0x10) as u32, 16).unwrap());
        ret.push(from_digit((*item & 0x0f) as u32, 16).unwrap());
    }
    ret
}

impl Store {
    /// Opens a new RocksDB at the specified location.
    pub fn open(path: &str, opts: StoreOptions) -> Store {
        let mut db_opts = rocksdb::Options::default();
        db_opts.create_if_missing(true);
        db_opts.set_compaction_style(DBCompactionStyle::Level);
        db_opts.set_max_open_files(256);
        db_opts.set_use_fsync(false);
        db_opts.set_compression_type(DBCompressionType::Snappy);
        db_opts.set_target_file_size_base(128 << 20);
        db_opts.set_write_buffer_size(256 << 20);
        // db_opts.set_compaction_readahead_size(2 << 20);
        db_opts.set_disable_auto_compactions(!opts.auto_compact);

        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_block_size(256 << 10);
        info!("opening {}", path);
        Store {
            db: DB::open(&db_opts, &path).unwrap(),
            rows: vec![],
            start: PreciseTime::now(),
        }
    }

    pub fn persist(&mut self, mut rows: Vec<Row>) {
        self.rows.append(&mut rows);
        let elapsed: Duration = self.start.to(PreciseTime::now());
        if elapsed < Duration::seconds(60) && self.rows.len() < 10_000_000 {
            return;
        }
        self.flush();
    }

    pub fn flush(&mut self) {
        let mut batch = WriteBatch::default();
        for row in &self.rows {
            batch.put(row.key.as_slice(), row.value.as_slice()).unwrap();
        }
        let mut opts = WriteOptions::new();
        opts.set_sync(true);
        self.db.write_opt(batch, &opts).unwrap();
        self.rows.clear();
        self.start = PreciseTime::now();
    }

    pub fn read_headers(&mut self) -> HashMap<String, BlockHeader> {
        let mut headers = HashMap::new();
        for row in self.scan(b"B") {
            let blockhash = revhex(&row.key);
            let header: BlockHeader = deserialize(&row.value).unwrap();
            headers.insert(blockhash, header);
        }
        headers
    }

    pub fn has_block(&mut self, blockhash: &[u8]) -> bool {
        let key: &[u8] = &[b"B", blockhash].concat();
        self.db.get(key).unwrap().is_some()
    }

    pub fn compact_if_needed(&mut self) {
        let key = b"F"; // full compaction
        if self.db.get(key).unwrap().is_some() {
            return;
        }
        info!("full compaction");
        self.db.compact_range(None, None); // should take a while
        self.db.put(key, b"").unwrap();
    }

    // Use generators ???
    fn scan(&mut self, prefix: &[u8]) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut iter = self.db.raw_iterator();
        let prefix_len = prefix.len();
        iter.seek(prefix);
        while iter.valid() {
            let key = &iter.key().unwrap();
            if &key[..prefix_len] != prefix {
                break;
            }
            rows.push(Row {
                key: key[prefix_len..].to_vec(),
                value: iter.value().unwrap().to_vec(),
            });
            iter.next();
        }
        rows
    }
}