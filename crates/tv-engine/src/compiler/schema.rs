use thiserror::Error;
use tv_core::{agg_alias, ColumnInfo, ViewOp};

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("unknown column: {0}")]
    UnknownColumn(String),
}

pub fn infer_schema(base: &[ColumnInfo], ops: &[ViewOp]) -> Result<Vec<ColumnInfo>, SchemaError> {
    let mut schema: Vec<ColumnInfo> = base.to_vec();
    for op in ops {
        schema = apply_op(schema, op)?;
    }
    Ok(schema)
}

pub fn infer_schema_step(
    schema: &[ColumnInfo],
    op: &ViewOp,
) -> Result<Vec<ColumnInfo>, SchemaError> {
    apply_op(schema.to_vec(), op)
}

fn apply_op(schema: Vec<ColumnInfo>, op: &ViewOp) -> Result<Vec<ColumnInfo>, SchemaError> {
    match op {
        ViewOp::Filter { .. } | ViewOp::Deduplicate { .. } | ViewOp::Sample { .. } => Ok(schema),

        ViewOp::Select { columns } => columns
            .iter()
            .enumerate()
            .map(|(new_idx, col_name)| {
                schema
                    .iter()
                    .find(|c| &c.name == col_name)
                    .map(|c| ColumnInfo {
                        index: new_idx,
                        ..c.clone()
                    })
                    .ok_or_else(|| SchemaError::UnknownColumn(col_name.clone()))
            })
            .collect(),

        ViewOp::Drop { columns } => {
            let excluded: std::collections::HashSet<&String> = columns.iter().collect();
            Ok(schema
                .into_iter()
                .filter(|c| !excluded.contains(&c.name))
                .enumerate()
                .map(|(i, mut c)| {
                    c.index = i;
                    c
                })
                .collect())
        }

        ViewOp::Sort { .. } => Ok(schema),

        ViewOp::Derive { name, .. } => {
            let new_idx = schema.len();
            let mut new_schema = schema;
            new_schema.push(ColumnInfo {
                index: new_idx,
                name: name.clone(),
                data_type: "unknown".to_string(),
                nullable: true,
            });
            Ok(new_schema)
        }

        ViewOp::Rename { mappings } => {
            let mapping_map: std::collections::HashMap<&String, &String> =
                mappings.iter().map(|(from, to)| (from, to)).collect();
            Ok(schema
                .into_iter()
                .map(|mut col| {
                    if let Some(new_name) = mapping_map.get(&col.name) {
                        col.name = (*new_name).clone();
                    }
                    col
                })
                .collect())
        }

        ViewOp::Limit { .. } => Ok(schema),

        ViewOp::GroupBy { keys, aggs } => {
            let key_infos: Vec<ColumnInfo> = keys
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    schema
                        .iter()
                        .find(|c| &c.name == k)
                        .map(|c| ColumnInfo {
                            index: i,
                            ..c.clone()
                        })
                        .ok_or_else(|| SchemaError::UnknownColumn(k.clone()))
                })
                .collect::<Result<_, _>>()?;

            let agg_infos: Vec<ColumnInfo> = aggs
                .iter()
                .enumerate()
                .map(|(i, agg)| ColumnInfo {
                    index: key_infos.len() + i,
                    name: agg_alias(agg).to_string(),
                    data_type: "unknown".to_string(),
                    nullable: true,
                })
                .collect();

            let mut result = key_infos;
            result.extend(agg_infos);
            Ok(result)
        }

        ViewOp::TopK { .. } | ViewOp::Approximate { .. } => Ok(schema),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tv_core::{AggExpr, ColumnInfo, Predicate, ScalarExpr, ViewOp};

    fn base_schema() -> Vec<ColumnInfo> {
        vec![
            ColumnInfo {
                index: 0,
                name: "id".into(),
                data_type: "Int64".into(),
                nullable: false,
            },
            ColumnInfo {
                index: 1,
                name: "name".into(),
                data_type: "Utf8".into(),
                nullable: true,
            },
            ColumnInfo {
                index: 2,
                name: "score".into(),
                data_type: "Float64".into(),
                nullable: true,
            },
        ]
    }

    #[test]
    fn infer_schema_filter() {
        let schema = base_schema();
        let ops = vec![ViewOp::Filter {
            predicate: Predicate::IsNotNull {
                column: "id".into(),
            },
        }];
        let result = infer_schema(&schema, &ops).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "id");
    }

    #[test]
    fn infer_schema_select() {
        let schema = base_schema();
        let ops = vec![ViewOp::Select {
            columns: vec!["id".into(), "score".into()],
        }];
        let result = infer_schema(&schema, &ops).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "id");
        assert_eq!(result[1].name, "score");
    }

    #[test]
    fn infer_schema_derive() {
        let schema = base_schema();
        let ops = vec![ViewOp::Derive {
            name: "doubled".into(),
            expr: ScalarExpr::Column {
                name: "score".into(),
            },
        }];
        let result = infer_schema(&schema, &ops).unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(result[3].name, "doubled");
    }

    #[test]
    fn infer_schema_group_by() {
        let schema = base_schema();
        let ops = vec![ViewOp::GroupBy {
            keys: vec!["name".into()],
            aggs: vec![AggExpr::Count {
                alias: "cnt".into(),
            }],
        }];
        let result = infer_schema(&schema, &ops).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "name");
        assert_eq!(result[1].name, "cnt");
    }

    #[test]
    fn infer_schema_rename() {
        let schema = base_schema();
        let ops = vec![ViewOp::Rename {
            mappings: vec![("name".into(), "label".into())],
        }];
        let result = infer_schema(&schema, &ops).unwrap();
        assert_eq!(result[1].name, "label");
    }

    #[test]
    fn infer_schema_drop() {
        let schema = base_schema();
        let ops = vec![ViewOp::Drop {
            columns: vec!["score".into()],
        }];
        let result = infer_schema(&schema, &ops).unwrap();
        assert_eq!(result.len(), 2);
        assert!(!result.iter().any(|c| c.name == "score"));
    }
}
