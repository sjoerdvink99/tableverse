use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

use tv_core::SortKey;

use crate::error::EngineError;

#[derive(Clone, Debug)]
struct IndexEntry {
    path: PathBuf,
    source_mtime: SystemTime,
    source_size: u64,
}

impl IndexEntry {
    fn is_valid_for(&self, source_path: &str) -> bool {
        match source_file_mtime_and_size(source_path) {
            Ok((mtime, size)) => mtime == self.source_mtime && size == self.source_size,
            Err(_) => false,
        }
    }
}

fn source_file_mtime_and_size(path: &str) -> Result<(SystemTime, u64), EngineError> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    Ok((mtime, meta.len()))
}

fn sort_key_fingerprint(keys: &[SortKey]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for k in keys {
        k.column.hash(&mut hasher);
        k.descending.hash(&mut hasher);
        k.nulls_last.hash(&mut hasher);
    }
    hasher.finish()
}

fn sort_index_path(source_path: &str, keys: &[SortKey]) -> PathBuf {
    let fp = sort_key_fingerprint(keys);
    PathBuf::from(format!("{}.sort_{:016x}.tvi", source_path, fp))
}

fn bloom_index_path(source_path: &str) -> PathBuf {
    PathBuf::from(format!("{}.tvb", source_path))
}

fn bitmap_index_path(source_path: &str, col_idx: usize) -> PathBuf {
    PathBuf::from(format!("{}.col_{}.tvd", source_path, col_idx))
}

fn sparse_sort_index_path(source_path: &str, keys: &[SortKey]) -> PathBuf {
    let fp = sort_key_fingerprint(keys);
    PathBuf::from(format!("{}.sparse_{:016x}.tvs", source_path, fp))
}

fn quantile_index_path(source_path: &str) -> PathBuf {
    PathBuf::from(format!("{}.tvq", source_path))
}

fn roaring_index_path(source_path: &str, col_idx: usize) -> PathBuf {
    PathBuf::from(format!("{}.col_{}.tvr", source_path, col_idx))
}

fn mark_index_path(source_path: &str, col_idx: usize) -> PathBuf {
    PathBuf::from(format!("{}.col_{}.tvk", source_path, col_idx))
}

pub struct IndexCatalog {
    sort_indexes: RwLock<HashMap<(String, u64), IndexEntry>>,
    bloom_indexes: RwLock<HashMap<String, IndexEntry>>,
    bitmap_indexes: RwLock<HashMap<(String, usize), IndexEntry>>,
    sparse_sort_indexes: RwLock<HashMap<(String, u64), IndexEntry>>,
    quantile_indexes: RwLock<HashMap<String, IndexEntry>>,
    roaring_indexes: RwLock<HashMap<(String, usize), IndexEntry>>,
    mark_indexes: RwLock<HashMap<(String, usize), IndexEntry>>,
}

impl Default for IndexCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexCatalog {
    pub fn new() -> Self {
        Self {
            sort_indexes: RwLock::new(HashMap::new()),
            bloom_indexes: RwLock::new(HashMap::new()),
            bitmap_indexes: RwLock::new(HashMap::new()),
            sparse_sort_indexes: RwLock::new(HashMap::new()),
            quantile_indexes: RwLock::new(HashMap::new()),
            roaring_indexes: RwLock::new(HashMap::new()),
            mark_indexes: RwLock::new(HashMap::new()),
        }
    }

    pub fn scan_for_source(&self, source_path: &str) {
        let parent = Path::new(source_path)
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let file_name = Path::new(source_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let Ok(entries) = std::fs::read_dir(parent) else {
            return;
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with(file_name) {
                continue;
            }

            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            if ext == "tvi" {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| {
                    s.strip_prefix(file_name)
                        .and_then(|s| s.strip_prefix(".sort_"))
                }) {
                    if let Ok(fp) = u64::from_str_radix(&stem[..stem.len().min(16)], 16) {
                        if let Ok((mtime, size)) = source_file_mtime_and_size(source_path) {
                            self.sort_indexes.write().unwrap().insert(
                                (source_path.to_string(), fp),
                                IndexEntry {
                                    path,
                                    source_mtime: mtime,
                                    source_size: size,
                                },
                            );
                        }
                    }
                }
            } else if ext == "tvb" {
                if let Ok((mtime, size)) = source_file_mtime_and_size(source_path) {
                    self.bloom_indexes.write().unwrap().insert(
                        source_path.to_string(),
                        IndexEntry {
                            path,
                            source_mtime: mtime,
                            source_size: size,
                        },
                    );
                }
            } else if ext == "tvs" {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| {
                    s.strip_prefix(file_name)
                        .and_then(|s| s.strip_prefix(".sparse_"))
                }) {
                    if let Ok(fp) = u64::from_str_radix(&stem[..stem.len().min(16)], 16) {
                        if let Ok((mtime, size)) = source_file_mtime_and_size(source_path) {
                            self.sparse_sort_indexes.write().unwrap().insert(
                                (source_path.to_string(), fp),
                                IndexEntry {
                                    path,
                                    source_mtime: mtime,
                                    source_size: size,
                                },
                            );
                        }
                    }
                }
            } else if ext == "tvq" {
                if let Ok((mtime, size)) = source_file_mtime_and_size(source_path) {
                    self.quantile_indexes.write().unwrap().insert(
                        source_path.to_string(),
                        IndexEntry {
                            path,
                            source_mtime: mtime,
                            source_size: size,
                        },
                    );
                }
            } else if ext == "tvd" {
                if let Some(col_part) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| {
                    s.strip_prefix(file_name)
                        .and_then(|s| s.strip_prefix(".col_"))
                }) {
                    if let Ok(col_idx) = col_part.parse::<usize>() {
                        if let Ok((mtime, size)) = source_file_mtime_and_size(source_path) {
                            self.bitmap_indexes.write().unwrap().insert(
                                (source_path.to_string(), col_idx),
                                IndexEntry {
                                    path,
                                    source_mtime: mtime,
                                    source_size: size,
                                },
                            );
                        }
                    }
                }
            } else if ext == "tvr" {
                if let Some(col_part) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| {
                    s.strip_prefix(file_name)
                        .and_then(|s| s.strip_prefix(".col_"))
                }) {
                    if let Ok(col_idx) = col_part.parse::<usize>() {
                        if let Ok((mtime, size)) = source_file_mtime_and_size(source_path) {
                            self.roaring_indexes.write().unwrap().insert(
                                (source_path.to_string(), col_idx),
                                IndexEntry {
                                    path,
                                    source_mtime: mtime,
                                    source_size: size,
                                },
                            );
                        }
                    }
                }
            } else if ext == "tvk" {
                if let Some(col_part) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| {
                    s.strip_prefix(file_name)
                        .and_then(|s| s.strip_prefix(".col_"))
                }) {
                    if let Ok(col_idx) = col_part.parse::<usize>() {
                        if let Ok((mtime, size)) = source_file_mtime_and_size(source_path) {
                            self.mark_indexes.write().unwrap().insert(
                                (source_path.to_string(), col_idx),
                                IndexEntry {
                                    path,
                                    source_mtime: mtime,
                                    source_size: size,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    pub fn lookup_sort_index(&self, source_path: &str, keys: &[SortKey]) -> Option<PathBuf> {
        let fp = sort_key_fingerprint(keys);
        let map = self.sort_indexes.read().unwrap();
        map.get(&(source_path.to_string(), fp))
            .filter(|e| e.is_valid_for(source_path))
            .map(|e| e.path.clone())
    }

    pub fn register_sort_index(
        &self,
        source_path: &str,
        keys: &[SortKey],
    ) -> Result<PathBuf, EngineError> {
        let fp = sort_key_fingerprint(keys);
        let path = sort_index_path(source_path, keys);
        let (mtime, size) = source_file_mtime_and_size(source_path)?;
        self.sort_indexes.write().unwrap().insert(
            (source_path.to_string(), fp),
            IndexEntry {
                path: path.clone(),
                source_mtime: mtime,
                source_size: size,
            },
        );
        Ok(path)
    }

    pub fn lookup_bloom_index(&self, source_path: &str) -> Option<PathBuf> {
        let map = self.bloom_indexes.read().unwrap();
        map.get(source_path)
            .filter(|e| e.is_valid_for(source_path))
            .map(|e| e.path.clone())
    }

    pub fn register_bloom_index(&self, source_path: &str) -> Result<PathBuf, EngineError> {
        let path = bloom_index_path(source_path);
        let (mtime, size) = source_file_mtime_and_size(source_path)?;
        self.bloom_indexes.write().unwrap().insert(
            source_path.to_string(),
            IndexEntry {
                path: path.clone(),
                source_mtime: mtime,
                source_size: size,
            },
        );
        Ok(path)
    }

    pub fn lookup_bitmap_index(&self, source_path: &str, col_idx: usize) -> Option<PathBuf> {
        let map = self.bitmap_indexes.read().unwrap();
        map.get(&(source_path.to_string(), col_idx))
            .filter(|e| e.is_valid_for(source_path))
            .map(|e| e.path.clone())
    }

    pub fn register_bitmap_index(
        &self,
        source_path: &str,
        col_idx: usize,
    ) -> Result<PathBuf, EngineError> {
        let path = bitmap_index_path(source_path, col_idx);
        let (mtime, size) = source_file_mtime_and_size(source_path)?;
        self.bitmap_indexes.write().unwrap().insert(
            (source_path.to_string(), col_idx),
            IndexEntry {
                path: path.clone(),
                source_mtime: mtime,
                source_size: size,
            },
        );
        Ok(path)
    }

    pub fn lookup_sparse_sort_index(&self, source_path: &str, keys: &[SortKey]) -> Option<PathBuf> {
        let fp = sort_key_fingerprint(keys);
        let map = self.sparse_sort_indexes.read().unwrap();
        map.get(&(source_path.to_string(), fp))
            .filter(|e| e.is_valid_for(source_path))
            .map(|e| e.path.clone())
    }

    pub fn register_sparse_sort_index(
        &self,
        source_path: &str,
        keys: &[SortKey],
    ) -> Result<PathBuf, EngineError> {
        let fp = sort_key_fingerprint(keys);
        let path = sparse_sort_index_path(source_path, keys);
        let (mtime, size) = source_file_mtime_and_size(source_path)?;
        self.sparse_sort_indexes.write().unwrap().insert(
            (source_path.to_string(), fp),
            IndexEntry {
                path: path.clone(),
                source_mtime: mtime,
                source_size: size,
            },
        );
        Ok(path)
    }

    pub fn lookup_quantile_index(&self, source_path: &str) -> Option<PathBuf> {
        let map = self.quantile_indexes.read().unwrap();
        map.get(source_path)
            .filter(|e| e.is_valid_for(source_path))
            .map(|e| e.path.clone())
    }

    pub fn register_quantile_index(&self, source_path: &str) -> Result<PathBuf, EngineError> {
        let path = quantile_index_path(source_path);
        let (mtime, size) = source_file_mtime_and_size(source_path)?;
        self.quantile_indexes.write().unwrap().insert(
            source_path.to_string(),
            IndexEntry {
                path: path.clone(),
                source_mtime: mtime,
                source_size: size,
            },
        );
        Ok(path)
    }

    pub fn lookup_roaring_index(&self, source_path: &str, col_idx: usize) -> Option<PathBuf> {
        let key = (source_path.to_string(), col_idx);
        let guard = self.roaring_indexes.read().unwrap();
        guard
            .get(&key)
            .filter(|e| e.is_valid_for(source_path))
            .map(|e| e.path.clone())
    }

    pub fn register_roaring_index(
        &self,
        source_path: &str,
        col_idx: usize,
    ) -> Result<PathBuf, EngineError> {
        let path = roaring_index_path(source_path, col_idx);
        let (mtime, size) = source_file_mtime_and_size(source_path)?;
        self.roaring_indexes.write().unwrap().insert(
            (source_path.to_string(), col_idx),
            IndexEntry {
                path: path.clone(),
                source_mtime: mtime,
                source_size: size,
            },
        );
        Ok(path)
    }

    pub fn lookup_mark_index(&self, source_path: &str, col_idx: usize) -> Option<PathBuf> {
        let key = (source_path.to_string(), col_idx);
        let guard = self.mark_indexes.read().unwrap();
        guard
            .get(&key)
            .filter(|e| e.is_valid_for(source_path))
            .map(|e| e.path.clone())
    }

    pub fn register_mark_index(
        &self,
        source_path: &str,
        col_idx: usize,
    ) -> Result<PathBuf, EngineError> {
        let path = mark_index_path(source_path, col_idx);
        let (mtime, size) = source_file_mtime_and_size(source_path)?;
        self.mark_indexes.write().unwrap().insert(
            (source_path.to_string(), col_idx),
            IndexEntry {
                path: path.clone(),
                source_mtime: mtime,
                source_size: size,
            },
        );
        Ok(path)
    }

    pub fn remove_source(&self, source_path: &str) {
        self.sort_indexes
            .write()
            .unwrap()
            .retain(|(path, _), _| path != source_path);
        self.bloom_indexes.write().unwrap().remove(source_path);
        self.bitmap_indexes
            .write()
            .unwrap()
            .retain(|(path, _), _| path != source_path);
        self.sparse_sort_indexes
            .write()
            .unwrap()
            .retain(|(path, _), _| path != source_path);
        self.quantile_indexes.write().unwrap().remove(source_path);
        self.roaring_indexes
            .write()
            .unwrap()
            .retain(|(path, _), _| path != source_path);
        self.mark_indexes
            .write()
            .unwrap()
            .retain(|(path, _), _| path != source_path);
    }
}
