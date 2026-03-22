use crate::error::EngineError;
use arrow::datatypes::{Schema, SchemaRef};
use arrow::ipc::writer::{IpcWriteOptions, StreamWriter};
use arrow::record_batch::RecordBatch;
use std::sync::Arc;

pub fn ipc_write_options() -> IpcWriteOptions {
    IpcWriteOptions::default()
}

pub fn serialize_empty_ipc(schema: SchemaRef) -> Result<Vec<u8>, EngineError> {
    let mut buf = Vec::new();
    {
        let mut writer =
            StreamWriter::try_new_with_options(&mut buf, &schema, ipc_write_options())?;
        writer.finish()?;
    }
    Ok(buf)
}

pub fn serialize_to_arrow_ipc(batches: &[RecordBatch]) -> Result<Vec<u8>, EngineError> {
    let non_empty: Vec<&RecordBatch> = batches.iter().filter(|b| b.num_rows() > 0).collect();
    if non_empty.is_empty() {
        let schema = batches
            .first()
            .map(|b| b.schema())
            .unwrap_or_else(|| Arc::new(Schema::empty()));
        return serialize_empty_ipc(schema);
    }
    let schema = non_empty[0].schema();
    let mut buf = Vec::new();
    {
        let mut writer =
            StreamWriter::try_new_with_options(&mut buf, &schema, ipc_write_options())?;
        for batch in &non_empty {
            writer.write(batch)?;
        }
        writer.finish()?;
    }
    Ok(buf)
}

pub fn project_tile_columns(
    batch: &RecordBatch,
    col_offset: usize,
    cols: usize,
) -> Result<RecordBatch, EngineError> {
    let schema = batch.schema();
    let col_end = (col_offset + cols).min(schema.fields().len());
    if col_offset >= schema.fields().len() {
        let empty_schema = Arc::new(Schema::empty());
        return Ok(RecordBatch::new_empty(empty_schema));
    }
    let indices: Vec<usize> = (col_offset..col_end).collect();
    Ok(batch.project(&indices)?)
}

pub fn maybe_dict_encode_batch(
    batch: &RecordBatch,
    dict_col_mask: &[bool],
) -> Result<RecordBatch, EngineError> {
    use arrow::datatypes::{DataType, Field};
    if dict_col_mask.iter().all(|&b| !b) {
        return Ok(batch.clone());
    }
    let schema = batch.schema();
    let dict_type = DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8));
    let mut new_fields: Vec<Field> = Vec::with_capacity(schema.fields().len());
    let mut new_cols: Vec<Arc<dyn arrow::array::Array>> = Vec::with_capacity(batch.num_columns());
    for (i, field) in schema.fields().iter().enumerate() {
        let col = batch.column(i);
        let should_encode = dict_col_mask.get(i).copied().unwrap_or(false)
            && matches!(field.data_type(), DataType::Utf8 | DataType::LargeUtf8);
        if should_encode {
            let utf8_col = if matches!(field.data_type(), DataType::LargeUtf8) {
                arrow::compute::cast(col.as_ref(), &DataType::Utf8)?
            } else {
                col.clone()
            };
            match arrow::compute::cast(utf8_col.as_ref(), &dict_type) {
                Ok(encoded) => {
                    new_fields.push(Field::new(
                        field.name(),
                        dict_type.clone(),
                        field.is_nullable(),
                    ));
                    new_cols.push(encoded);
                }
                Err(_) => {
                    new_fields.push(field.as_ref().clone());
                    new_cols.push(col.clone());
                }
            }
        } else {
            new_fields.push(field.as_ref().clone());
            new_cols.push(col.clone());
        }
    }
    let new_schema = Arc::new(Schema::new(new_fields));
    Ok(RecordBatch::try_new(new_schema, new_cols)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Int32Array;
    use arrow::datatypes::{DataType, Field};

    #[test]
    fn roundtrip_empty() {
        let buf = serialize_to_arrow_ipc(&[]).unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn roundtrip_batch() {
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![1, 2, 3]))]).unwrap();
        let buf = serialize_to_arrow_ipc(&[batch]).unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn roundtrip_zero_rows_with_schema() {
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let batch = RecordBatch::new_empty(schema.clone());
        let buf = serialize_to_arrow_ipc(&[batch]).unwrap();
        assert!(!buf.is_empty());
        let reader =
            arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(&buf), None).unwrap();
        assert_eq!(reader.schema().fields().len(), 1);
    }

    #[test]
    fn project_tile_columns_middle() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int32, false),
            Field::new("b", DataType::Int32, false),
            Field::new("c", DataType::Int32, false),
            Field::new("d", DataType::Int32, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![1i32])),
                Arc::new(Int32Array::from(vec![2i32])),
                Arc::new(Int32Array::from(vec![3i32])),
                Arc::new(Int32Array::from(vec![4i32])),
            ],
        )
        .unwrap();
        let projected = project_tile_columns(&batch, 1, 2).unwrap();
        assert_eq!(projected.num_columns(), 2);
        assert_eq!(projected.schema().field(0).name(), "b");
        assert_eq!(projected.schema().field(1).name(), "c");
    }

    #[test]
    fn project_tile_columns_beyond_end() {
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![1i32]))]).unwrap();
        let projected = project_tile_columns(&batch, 0, 10).unwrap();
        assert_eq!(projected.num_columns(), 1);
    }

    #[test]
    fn serialize_empty_ipc_has_schema() {
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let buf = serialize_empty_ipc(schema.clone()).unwrap();
        assert!(!buf.is_empty());
        let reader =
            arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(&buf), None).unwrap();
        assert_eq!(reader.schema().fields().len(), 1);
    }

    #[test]
    fn serialize_to_arrow_ipc_filters_empty() {
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let empty = RecordBatch::new_empty(schema.clone());
        let real =
            RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![1i32, 2]))]).unwrap();
        let buf = serialize_to_arrow_ipc(&[empty, real]).unwrap();
        assert!(!buf.is_empty());
    }
}
