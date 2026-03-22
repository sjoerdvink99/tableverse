use arrow::ipc::reader::StreamReader;
use arrow::record_batch::RecordBatch;
use arrow_flight::flight_service_client::FlightServiceClient;
use arrow_flight::{FlightDescriptor, Ticket};
use futures::StreamExt;
use tonic::transport::Channel;

use crate::descriptor::{encode_get_descriptor, GetDescriptor};
use crate::error::FlightError;

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024 * 1024;

pub struct FlightClient {
    inner: FlightServiceClient<Channel>,
}

impl FlightClient {
    pub async fn connect(host: &str, port: u16) -> Result<Self, FlightError> {
        let endpoint = format!("http://{host}:{port}");
        let channel = Channel::from_shared(endpoint)
            .map_err(|e| FlightError::InvalidDescriptor(format!("invalid endpoint: {e}")))?
            .connect()
            .await?;
        Ok(Self {
            inner: FlightServiceClient::new(channel),
        })
    }

    pub async fn get_source_schema(
        &mut self,
        source_id: &str,
    ) -> Result<arrow::datatypes::SchemaRef, FlightError> {
        let desc = GetDescriptor {
            source_id: source_id.to_string(),
            view_expr_bytes: None,
            row: 0,
            col: 0,
            rows: 0,
            cols: 0,
        };
        let ticket_bytes = encode_get_descriptor(&desc)?;
        let fd = FlightDescriptor {
            r#type: 0,
            cmd: ticket_bytes,
            path: vec![],
        };
        let info = self
            .inner
            .get_flight_info(tonic::Request::new(fd))
            .await?
            .into_inner();

        let schema = arrow::ipc::convert::try_schema_from_flatbuffer_bytes(&info.schema)?;
        Ok(std::sync::Arc::new(schema))
    }

    pub async fn fetch_batches(
        &mut self,
        source_id: &str,
        row_limit: u64,
    ) -> Result<Vec<RecordBatch>, FlightError> {
        let desc = GetDescriptor {
            source_id: source_id.to_string(),
            view_expr_bytes: None,
            row: 0,
            col: 0,
            rows: row_limit,
            cols: 0,
        };
        let ticket_bytes = encode_get_descriptor(&desc)?;

        let mut stream = self
            .inner
            .do_get(tonic::Request::new(Ticket {
                ticket: ticket_bytes,
            }))
            .await?
            .into_inner();

        let mut ipc_bytes: Vec<u8> = vec![];
        while let Some(chunk) = stream.next().await {
            let fd = chunk?;
            let incoming = fd.data_header.len() + fd.data_body.len();
            if ipc_bytes.len() + incoming > MAX_RESPONSE_BYTES {
                return Err(FlightError::InvalidDescriptor(format!(
                    "response exceeds maximum size of {} bytes",
                    MAX_RESPONSE_BYTES
                )));
            }
            ipc_bytes.extend_from_slice(&fd.data_header);
            ipc_bytes.extend_from_slice(&fd.data_body);
        }

        let cursor = std::io::Cursor::new(ipc_bytes);
        let reader = StreamReader::try_new(cursor, None)?;
        Ok(reader.collect::<Result<Vec<_>, _>>()?)
    }
}

pub fn parse_flight_uri(uri: &str) -> Result<(String, u16, String), FlightError> {
    let stripped = uri.strip_prefix("flight://").ok_or_else(|| {
        FlightError::InvalidDescriptor(format!(
            "expected flight:// URI (e.g. flight://localhost:8080/source_id), got: {uri}"
        ))
    })?;

    let (host_port, source_id) = stripped.split_once('/').ok_or_else(|| {
        FlightError::InvalidDescriptor(format!(
            "missing source_id in URI (e.g. flight://localhost:8080/source_id), got: {uri}"
        ))
    })?;

    let (host, port_str) = host_port.rsplit_once(':').ok_or_else(|| {
        FlightError::InvalidDescriptor(format!(
            "missing port in URI (e.g. flight://localhost:8080/source_id), got: {uri}"
        ))
    })?;

    let port = port_str.parse::<u16>().map_err(|_| {
        FlightError::InvalidDescriptor(format!(
            "invalid port '{port_str}' in URI (must be 1–65535), got: {uri}"
        ))
    })?;

    if source_id.is_empty() {
        return Err(FlightError::InvalidDescriptor(format!(
            "empty source_id in URI (e.g. flight://localhost:8080/source_id), got: {uri}"
        )));
    }

    Ok((host.to_string(), port, source_id.to_string()))
}
