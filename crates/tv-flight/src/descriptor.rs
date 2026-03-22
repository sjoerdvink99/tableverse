use arrow_flight::FlightDescriptor;
use serde::{Deserialize, Serialize};

use crate::error::FlightError;

const DEFAULT_TILE_ROWS: u64 = 256;
const DEFAULT_TILE_COLS: u64 = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutDescriptor {
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GetDescriptor {
    pub source_id: String,
    pub view_expr_bytes: Option<Vec<u8>>,
    pub row: u64,
    pub col: usize,
    pub rows: u64,
    pub cols: usize,
}

pub fn parse_put_descriptor(descriptor: &FlightDescriptor) -> Result<PutDescriptor, FlightError> {
    if descriptor.cmd.is_empty() {
        return Ok(PutDescriptor { name: None });
    }
    serde_json::from_slice(&descriptor.cmd)
        .map_err(|e| FlightError::InvalidDescriptor(format!("invalid put descriptor: {e}")))
}

pub fn parse_get_descriptor(descriptor: &FlightDescriptor) -> Result<GetDescriptor, FlightError> {
    if descriptor.cmd.is_empty() {
        return Err(FlightError::InvalidDescriptor(
            "empty get descriptor".to_string(),
        ));
    }
    let v: serde_json::Value = serde_json::from_slice(&descriptor.cmd)
        .map_err(|e| FlightError::InvalidDescriptor(format!("invalid get descriptor JSON: {e}")))?;

    let source_id = v["source_id"]
        .as_str()
        .ok_or_else(|| {
            FlightError::InvalidDescriptor(
                "missing or non-string 'source_id' field in descriptor".to_string(),
            )
        })?
        .to_string();

    let view_expr_bytes = v
        .as_object()
        .and_then(|o| o.get("view_expr"))
        .filter(|ve| !ve.is_null())
        .map(|ve| ve.to_string().into_bytes());

    Ok(GetDescriptor {
        source_id,
        view_expr_bytes,
        row: v["row"].as_u64().unwrap_or(0),
        col: v["col"].as_u64().unwrap_or(0) as usize,
        rows: v["rows"].as_u64().unwrap_or(DEFAULT_TILE_ROWS),
        cols: v["cols"].as_u64().unwrap_or(DEFAULT_TILE_COLS) as usize,
    })
}

pub fn encode_get_descriptor(desc: &GetDescriptor) -> Result<bytes::Bytes, FlightError> {
    let view_expr_val = desc
        .view_expr_bytes
        .as_deref()
        .map(serde_json::from_slice::<serde_json::Value>)
        .transpose()
        .map_err(|e| FlightError::InvalidDescriptor(format!("invalid view_expr JSON: {e}")))?
        .unwrap_or(serde_json::Value::Null);

    let json = serde_json::json!({
        "source_id": desc.source_id,
        "view_expr": view_expr_val,
        "row": desc.row,
        "col": desc.col,
        "rows": desc.rows,
        "cols": desc.cols,
    });

    Ok(bytes::Bytes::from(serde_json::to_vec(&json)?))
}
