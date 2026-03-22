use std::fs::File;
use std::vec;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::{ArrowPredicateFn, ParquetRecordBatchReaderBuilder, RowFilter};

use tv_core::{Predicate, SourceFormat, SourceKind, SourceMeta, ViewOp};

use crate::error::EngineError;
use crate::executor::predicate_to_bool_array;
use crate::reader::{self, predicate_column_indices};

pub enum BatchStream {
    Parquet(ParquetBatchStream),
    Multi(MultiBatchStream),
    InMemory(vec::IntoIter<RecordBatch>),
}

pub struct ParquetBatchStream {
    reader: parquet::arrow::arrow_reader::ParquetRecordBatchReader,
}

pub struct MultiBatchStream {
    files: Vec<String>,
    file_idx: usize,
    current: Option<parquet::arrow::arrow_reader::ParquetRecordBatchReader>,
    predicate: Option<Predicate>,
    schema: SchemaRef,
}

impl Iterator for BatchStream {
    type Item = Result<RecordBatch, EngineError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            BatchStream::Parquet(s) => s.next(),
            BatchStream::Multi(s) => s.next(),
            BatchStream::InMemory(it) => it.next().map(Ok),
        }
    }
}

impl Iterator for ParquetBatchStream {
    type Item = Result<RecordBatch, EngineError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.next().map(|r| r.map_err(EngineError::Arrow))
    }
}

impl Iterator for MultiBatchStream {
    type Item = Result<RecordBatch, EngineError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut rdr) = self.current {
                if let Some(r) = rdr.next() {
                    return Some(r.map_err(EngineError::Arrow));
                }
                self.current = None;
                self.file_idx += 1;
            }
            if self.file_idx >= self.files.len() {
                return None;
            }
            match open_parquet_reader(
                &self.files[self.file_idx],
                self.predicate.as_ref(),
                &self.schema,
            ) {
                Ok(rdr) => self.current = Some(rdr),
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

fn open_parquet_reader(
    path: &str,
    predicate: Option<&Predicate>,
    schema: &SchemaRef,
) -> Result<parquet::arrow::arrow_reader::ParquetRecordBatchReader, EngineError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    if let Some(pred) = predicate {
        let pred_col_indices = predicate_column_indices(pred, schema);
        let pred_mask = parquet::arrow::ProjectionMask::roots(
            builder.parquet_schema(),
            pred_col_indices.iter().copied(),
        );
        let cloned = pred.clone();
        let row_filter = RowFilter::new(vec![Box::new(ArrowPredicateFn::new(
            pred_mask,
            move |batch| {
                predicate_to_bool_array(&batch, &cloned)
                    .map_err(|e| arrow::error::ArrowError::ExternalError(Box::new(e)))
            },
        ))]);
        Ok(builder.with_row_filter(row_filter).build()?)
    } else {
        Ok(builder.build()?)
    }
}

pub fn stream_source(
    meta: &SourceMeta,
    ops: &[ViewOp],
    schema_hint: Option<SchemaRef>,
) -> Result<BatchStream, EngineError> {
    let filter_pred = extract_combined_filter(ops);

    if matches!(meta.format, SourceFormat::Parquet) && !is_cloud_kind(&meta.kind) {
        if meta.files.len() <= 1 {
            let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
            let schema = match schema_hint {
                Some(s) => s,
                None => reader::parquet_schema_and_rows(path).map(|(s, _)| s)?,
            };
            let file = File::open(path)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
            let reader = if let Some(pred) = filter_pred {
                let pred_col_indices = predicate_column_indices(&pred, &schema);
                let pred_mask = parquet::arrow::ProjectionMask::roots(
                    builder.parquet_schema(),
                    pred_col_indices.iter().copied(),
                );
                let cloned = pred.clone();
                let row_filter = RowFilter::new(vec![Box::new(ArrowPredicateFn::new(
                    pred_mask,
                    move |batch| {
                        predicate_to_bool_array(&batch, &cloned)
                            .map_err(|e| arrow::error::ArrowError::ExternalError(Box::new(e)))
                    },
                ))]);
                builder.with_row_filter(row_filter).build()?
            } else {
                builder.build()?
            };
            return Ok(BatchStream::Parquet(ParquetBatchStream { reader }));
        } else {
            let schema = match schema_hint {
                Some(s) => s,
                None => {
                    let path = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
                    reader::parquet_schema_and_rows(path).map(|(s, _)| s)?
                }
            };
            return Ok(BatchStream::Multi(MultiBatchStream {
                files: meta.files.clone(),
                file_idx: 0,
                current: None,
                predicate: filter_pred,
                schema,
            }));
        }
    }

    let uri = meta.files.first().map(|s| s.as_str()).unwrap_or(&meta.uri);
    let batches = reader::read_source_full(uri, &meta.format, None)?;
    Ok(BatchStream::InMemory(batches.into_iter()))
}

fn extract_combined_filter(ops: &[ViewOp]) -> Option<Predicate> {
    let preds: Vec<Predicate> = ops
        .iter()
        .filter_map(|op| {
            if let ViewOp::Filter { predicate } = op {
                Some(predicate.clone())
            } else {
                None
            }
        })
        .collect();
    match preds.len() {
        0 => None,
        1 => Some(preds.into_iter().next().unwrap()),
        _ => Some(Predicate::And { exprs: preds }),
    }
}

fn is_cloud_kind(kind: &SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::S3 | SourceKind::Gcs | SourceKind::AzureBlob | SourceKind::Http
    )
}
