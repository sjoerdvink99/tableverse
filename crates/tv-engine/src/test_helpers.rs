use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{Array, BooleanArray, Float64Array, Int32Array, Int64Array, StringArray};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

pub fn int_string_batch(rows: usize) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", ArrowDataType::Int64, false),
        Field::new("name", ArrowDataType::Utf8, false),
        Field::new("score", ArrowDataType::Float64, false),
        Field::new("flag", ArrowDataType::Boolean, false),
    ]));
    let ids: Vec<i64> = (0..rows as i64).collect();
    let names: Vec<String> = (0..rows).map(|i| format!("item_{i}")).collect();
    let name_strs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let scores: Vec<f64> = (0..rows).map(|i| i as f64 * 1.1).collect();
    let flags: Vec<bool> = (0..rows).map(|i| i % 2 == 0).collect();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(name_strs)),
            Arc::new(Float64Array::from(scores)),
            Arc::new(BooleanArray::from(flags)),
        ],
    )
    .unwrap()
}

pub fn nullable_batch(rows: usize) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", ArrowDataType::Int64, false),
        Field::new("name", ArrowDataType::Utf8, true),
        Field::new("score", ArrowDataType::Float64, true),
        Field::new("flag", ArrowDataType::Boolean, true),
    ]));
    let ids: Vec<i64> = (0..rows as i64).collect();
    let names: Vec<Option<String>> = (0..rows)
        .map(|i| {
            if i % 3 == 0 {
                None
            } else {
                Some(format!("item_{i}"))
            }
        })
        .collect();
    let name_opts: Vec<Option<&str>> = names.iter().map(|s| s.as_deref()).collect();
    let scores: Vec<Option<f64>> = (0..rows)
        .map(|i| {
            if i % 3 == 0 {
                None
            } else {
                Some(i as f64 * 1.1)
            }
        })
        .collect();
    let flags: Vec<Option<bool>> = (0..rows)
        .map(|i| if i % 3 == 0 { None } else { Some(i % 2 == 0) })
        .collect();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(name_opts)),
            Arc::new(Float64Array::from(scores)),
            Arc::new(BooleanArray::from(flags)),
        ],
    )
    .unwrap()
}

pub fn single_column_batch(name: &str, values: &[f64]) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![Field::new(
        name,
        ArrowDataType::Float64,
        false,
    )]));
    RecordBatch::try_new(schema, vec![Arc::new(Float64Array::from(values.to_vec()))]).unwrap()
}

pub fn single_string_column_batch(name: &str, values: &[&str]) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![Field::new(
        name,
        ArrowDataType::Utf8,
        false,
    )]));
    RecordBatch::try_new(schema, vec![Arc::new(StringArray::from(values.to_vec()))]).unwrap()
}

pub fn empty_batch(schema: SchemaRef) -> RecordBatch {
    let cols: Vec<Arc<dyn Array>> = schema
        .fields()
        .iter()
        .map(|f| arrow::array::new_empty_array(f.data_type()))
        .collect();
    RecordBatch::try_new(schema, cols).unwrap()
}

pub fn write_test_parquet(dir: &tempfile::TempDir, name: &str, batches: &[RecordBatch]) -> PathBuf {
    write_multi_rg_parquet(dir, name, batches, 50)
}

pub fn write_multi_rg_parquet(
    dir: &tempfile::TempDir,
    name: &str,
    batches: &[RecordBatch],
    rg_size: usize,
) -> PathBuf {
    let path = dir.path().join(name);
    let file = std::fs::File::create(&path).unwrap();
    let props = WriterProperties::builder()
        .set_max_row_group_size(rg_size)
        .build();
    if batches.is_empty() {
        return path;
    }
    let schema = batches[0].schema();
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).unwrap();
    for batch in batches {
        writer.write(batch).unwrap();
    }
    writer.close().unwrap();
    path
}

pub fn people_batches() -> Vec<RecordBatch> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", ArrowDataType::Int64, false),
        Field::new("name", ArrowDataType::Utf8, false),
        Field::new("age", ArrowDataType::Int64, false),
        Field::new("salary", ArrowDataType::Float64, false),
        Field::new("department", ArrowDataType::Utf8, false),
        Field::new("active", ArrowDataType::Boolean, false),
    ]));
    let depts = ["Engineering", "Sales", "HR", "Finance"];
    let n = 200;
    let ids: Vec<i64> = (0..n as i64).collect();
    let names: Vec<String> = (0..n).map(|i| format!("Person{i}")).collect();
    let name_strs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let ages: Vec<i64> = (0..n as i64).map(|i| 22 + (i % 43)).collect();
    let salaries: Vec<f64> = (0..n).map(|i| 40000.0 + (i as f64) * 300.0).collect();
    let dept_strs: Vec<&str> = (0..n).map(|i| depts[i % 4]).collect();
    let actives: Vec<bool> = (0..n).map(|i| i % 5 != 0).collect();
    vec![RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(name_strs)),
            Arc::new(Int64Array::from(ages)),
            Arc::new(Float64Array::from(salaries)),
            Arc::new(StringArray::from(dept_strs)),
            Arc::new(BooleanArray::from(actives)),
        ],
    )
    .unwrap()]
}

pub fn numbers_batches() -> Vec<RecordBatch> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", ArrowDataType::Float64, false),
        Field::new("y", ArrowDataType::Float64, false),
        Field::new("z", ArrowDataType::Float64, false),
        Field::new("label", ArrowDataType::Utf8, false),
    ]));
    let n = 1000;
    let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let ys: Vec<f64> = (0..n).map(|i| i as f64 * 2.0 + 1.0).collect();
    let zs: Vec<f64> = (0..n).map(|i| (i as f64).sin()).collect();
    let labels: Vec<String> = (0..n).map(|i| format!("L{}", i % 10)).collect();
    let label_strs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    vec![RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
            Arc::new(Float64Array::from(zs)),
            Arc::new(StringArray::from(label_strs)),
        ],
    )
    .unwrap()]
}

pub fn multi_type_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("i32_col", ArrowDataType::Int32, false),
        Field::new("i64_col", ArrowDataType::Int64, false),
        Field::new("f64_col", ArrowDataType::Float64, false),
        Field::new("str_col", ArrowDataType::Utf8, false),
        Field::new("bool_col", ArrowDataType::Boolean, false),
    ]));
    let n = 10;
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from((0..n as i32).collect::<Vec<_>>())),
            Arc::new(Int64Array::from((0..n as i64).collect::<Vec<_>>())),
            Arc::new(Float64Array::from(
                (0..n).map(|i| i as f64 * 0.5).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                (0..n)
                    .map(|i| format!("s{i}"))
                    .collect::<Vec<_>>()
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(BooleanArray::from(
                (0..n).map(|i| i % 2 == 0).collect::<Vec<_>>(),
            )),
        ],
    )
    .unwrap()
}
