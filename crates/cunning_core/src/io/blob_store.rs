use std::sync::{Arc, Mutex, OnceLock};
use redb::{Database, ReadableTable, TableDefinition};
use std::path::PathBuf;

// Table: ID (u64) -> Data (Vec<u8>)
// We use a u64 ID to look up blobs. 
// In a content-addressable system, we might look up by hash (u128/bytes), 
// but for the internal handle system, u64 is faster and compatible with FFI Handle type.
const BLOBS_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("blobs");
const META_TABLE: TableDefinition<u64, u64> = TableDefinition::new("meta");
const META_NEXT_ID_KEY: u64 = 0;
// Table: Key (u64) -> Latest blob id (u64)
// Used for lightweight "channel" pointers, e.g. viewport.unity / viewport.cunning.
const LATEST_TABLE: TableDefinition<u64, u64> = TableDefinition::new("latest");

// Optional: Hash -> ID lookup for deduplication
// const HASH_INDEX: TableDefinition<&[u8; 32], u64> = TableDefinition::new("hash_index");

pub struct BlobStore {
    db: Arc<Database>,
}

impl BlobStore {
    pub fn open(path: PathBuf) -> Result<Self, redb::Error> { Self::open_or_create(path) }

    pub fn open_existing(path: PathBuf) -> Result<Self, redb::Error> { Self::open_impl(path, false) }

    pub fn open_or_create(path: PathBuf) -> Result<Self, redb::Error> {
        Self::open_existing(path.clone()).or_else(|_| Self::open_impl(path, true))
    }

    fn open_impl(path: PathBuf, create: bool) -> Result<Self, redb::Error> {
        let db = if create { Database::create(path)? } else { Database::open(path)? };
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(BLOBS_TABLE)?;
            let mut meta = write_txn.open_table(META_TABLE)?;
            if meta.get(META_NEXT_ID_KEY)?.is_none() { meta.insert(META_NEXT_ID_KEY, 1u64)?; }
            let _ = write_txn.open_table(LATEST_TABLE)?;
        }
        write_txn.commit()?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Insert a blob and return its size.
    pub fn insert(&self, id: u64, data: &[u8]) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(BLOBS_TABLE)?;
            table.insert(id, data)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn insert_alloc(&self, data: &[u8]) -> Result<u64, redb::Error> {
        let write_txn = self.db.begin_write()?;
        let id = {
            let mut meta = write_txn.open_table(META_TABLE)?;
            let next = meta.get(META_NEXT_ID_KEY)?.map(|v| v.value()).unwrap_or(1);
            meta.insert(META_NEXT_ID_KEY, next.saturating_add(1))?;
            next
        };
        {
            let mut table = write_txn.open_table(BLOBS_TABLE)?;
            table.insert(id, data)?;
        }
        write_txn.commit()?;
        Ok(id)
    }

    /// Get a blob copy.
    /// Note: redb uses mmap, so the read from disk is paged in by OS.
    /// Copying to Vec<u8> is necessary to pass ownership unless we expose redb's AccessGuard.
    pub fn get(&self, id: u64) -> Result<Option<Vec<u8>>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(BLOBS_TABLE)?;
        let value = table.get(id)?;
        Ok(value.map(|v| v.value().to_vec()))
    }

    pub fn set_latest(&self, key: u64, blob_id: u64) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        { let mut t = write_txn.open_table(LATEST_TABLE)?; t.insert(key, blob_id)?; }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_latest(&self, key: u64) -> Result<u64, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let t = read_txn.open_table(LATEST_TABLE)?;
        Ok(t.get(key)?.map(|v| v.value()).unwrap_or(0))
    }
    
    // For "Zero-Copy" FFI:
    // We can't easily return a pointer to redb's internal mmap because of lifetime constraints (AccessGuard).
    // However, if we need extreme performance for read-only access in Rust, we can pass the AccessGuard around.
}

static GLOBAL_STORE: OnceLock<Mutex<BlobStore>> = OnceLock::new();

pub fn init_global_store(path: PathBuf) {
    let store = BlobStore::open_or_create(path).expect("Failed to open global blob store");
    GLOBAL_STORE.set(Mutex::new(store)).ok();
}

pub fn init_global_store_existing(path: PathBuf) {
    let store = BlobStore::open_existing(path).expect("Failed to open existing global blob store");
    GLOBAL_STORE.set(Mutex::new(store)).ok();
}

pub fn global_store() -> Option<std::sync::MutexGuard<'static, BlobStore>> {
    GLOBAL_STORE.get().map(|m| m.lock().unwrap())
}
