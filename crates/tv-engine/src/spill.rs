use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::RecordBatchReader;
use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use crate::error::EngineError;

#[derive(Clone)]
pub struct SpilledRun {
    pub path: PathBuf,
    pub row_count: u64,
}

impl SpilledRun {
    pub fn file_size(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

pub struct SpillWriter {
    pub dir: PathBuf,
    schema: SchemaRef,
    run_count: usize,
}

impl SpillWriter {
    pub fn new(dir: PathBuf, schema: SchemaRef) -> Self {
        Self {
            dir,
            schema,
            run_count: 0,
        }
    }

    pub fn dir_path(&self) -> &std::path::Path {
        &self.dir
    }

    pub fn write_run(&mut self, batches: &[RecordBatch]) -> Result<SpilledRun, EngineError> {
        let path = self.dir.join(format!("run_{:04}.parquet", self.run_count));
        self.run_count += 1;

        let file = File::create(&path)?;
        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .set_max_row_group_size(65536)
            .build();
        let mut writer = ArrowWriter::try_new(file, self.schema.clone(), Some(props))?;

        let mut row_count = 0u64;
        for batch in batches {
            writer.write(batch)?;
            row_count += batch.num_rows() as u64;
        }
        writer.close()?;

        Ok(SpilledRun { path, row_count })
    }
}

pub struct SpillReader {
    reader: parquet::arrow::arrow_reader::ParquetRecordBatchReader,
}

impl SpillReader {
    pub fn open(path: &Path) -> Result<Self, EngineError> {
        let file = File::open(path)?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)?
            .with_batch_size(4096)
            .build()?;
        Ok(Self { reader })
    }

    pub fn open_with_projection(path: &Path, col_indices: &[usize]) -> Result<Self, EngineError> {
        let file = File::open(path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        let mask = parquet::arrow::ProjectionMask::roots(
            builder.parquet_schema(),
            col_indices.iter().copied(),
        );
        let reader = builder
            .with_projection(mask)
            .with_batch_size(4096)
            .build()?;
        Ok(Self { reader })
    }

    pub fn schema(&self) -> Arc<arrow::datatypes::Schema> {
        self.reader.schema()
    }
}

impl Iterator for SpillReader {
    type Item = Result<RecordBatch, EngineError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.next().map(|r| r.map_err(EngineError::Arrow))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::int_string_batch;
    use tempfile::TempDir;

    #[test]
    fn spill_write_read_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(20);
        let schema = batch.schema();
        let mut writer = SpillWriter::new(tmp.path().to_path_buf(), schema);
        let run = writer.write_run(&[batch.clone()]).unwrap();
        assert_eq!(run.row_count, 20);

        let reader = SpillReader::open(&run.path).unwrap();
        let batches: Vec<RecordBatch> = reader.collect::<Result<_, _>>().unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 20);
    }

    #[test]
    fn spill_write_empty_batches() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(0);
        let schema = batch.schema();
        let mut writer = SpillWriter::new(tmp.path().to_path_buf(), schema);
        let run = writer.write_run(&[batch]).unwrap();
        assert_eq!(run.row_count, 0);

        let reader = SpillReader::open(&run.path).unwrap();
        let batches: Vec<RecordBatch> = reader.collect::<Result<_, _>>().unwrap();
        assert!(batches.iter().all(|b| b.num_rows() == 0));
    }

    #[test]
    fn spill_reader_with_projection() {
        let tmp = TempDir::new().unwrap();
        let batch = int_string_batch(10);
        let schema = batch.schema();
        let mut writer = SpillWriter::new(tmp.path().to_path_buf(), schema);
        let run = writer.write_run(&[batch]).unwrap();

        let reader = SpillReader::open_with_projection(&run.path, &[0, 2]).unwrap();
        assert_eq!(reader.schema().fields().len(), 2);
        let batches: Vec<RecordBatch> = reader.collect::<Result<_, _>>().unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 10);
        for batch in &batches {
            assert_eq!(batch.num_columns(), 2);
        }
    }
}
