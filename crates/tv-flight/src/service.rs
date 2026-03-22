use std::pin::Pin;
use std::sync::Arc;

use arrow::error::ArrowError;
use arrow::ipc::reader::StreamReader;
use arrow::ipc::writer::{DictionaryTracker, IpcDataGenerator, IpcWriteOptions};
use arrow_flight::flight_service_server::FlightService;
use arrow_flight::{
    Action, ActionType, Criteria, Empty, FlightData, FlightDescriptor, FlightInfo,
    HandshakeRequest, HandshakeResponse, PollInfo, PutResult, SchemaResult, Ticket,
};
use futures::{Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::{info, warn};
use tv_core::SourceMeta;

use crate::descriptor::{
    encode_get_descriptor, parse_get_descriptor, parse_put_descriptor, GetDescriptor,
};

type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

const MAX_LIST_SOURCES: usize = 1000;

#[derive(Clone)]
pub struct TableverseFlightService {
    engine: Arc<tv_engine::Engine>,
}

impl TableverseFlightService {
    pub fn new(engine: Arc<tv_engine::Engine>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl FlightService for TableverseFlightService {
    type HandshakeStream = BoxStream<HandshakeResponse>;
    type ListFlightsStream = BoxStream<FlightInfo>;
    type DoGetStream = BoxStream<FlightData>;
    type DoPutStream = BoxStream<PutResult>;
    type DoExchangeStream = BoxStream<FlightData>;
    type DoActionStream = BoxStream<arrow_flight::Result>;
    type ListActionsStream = BoxStream<ActionType>;

    async fn handshake(
        &self,
        _request: Request<Streaming<HandshakeRequest>>,
    ) -> Result<Response<Self::HandshakeStream>, Status> {
        let stream = futures::stream::empty();
        Ok(Response::new(Box::pin(stream)))
    }

    async fn list_flights(
        &self,
        _request: Request<Criteria>,
    ) -> Result<Response<Self::ListFlightsStream>, Status> {
        let sources = self.engine.list_sources();
        let infos: Vec<Result<FlightInfo, Status>> = sources
            .into_iter()
            .take(MAX_LIST_SOURCES)
            .map(|meta| source_to_flight_info(&meta))
            .collect();

        let stream = futures::stream::iter(infos);
        Ok(Response::new(Box::pin(stream)))
    }

    async fn get_flight_info(
        &self,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let descriptor = request.into_inner();
        let desc = parse_get_descriptor(&descriptor).map_err(Status::from)?;

        let meta = self
            .engine
            .get_source(&desc.source_id)
            .ok_or_else(|| Status::not_found(format!("source '{}' not found", desc.source_id)))?;

        let endpoint_bytes = encode_get_descriptor(&desc).map_err(Status::from)?;
        let schema_bytes = schema_bytes_from_source(&meta)
            .map_err(|e| Status::internal(format!("schema serialization failed: {e}")))?;
        let total_records = meta.n_rows as i64;
        let total_bytes = meta.file_size_bytes.min(i64::MAX as u64) as i64;

        let info = FlightInfo {
            schema: schema_bytes,
            flight_descriptor: Some(descriptor),
            endpoint: vec![arrow_flight::FlightEndpoint {
                ticket: Some(Ticket {
                    ticket: endpoint_bytes,
                }),
                location: vec![],
                expiration_time: None,
                app_metadata: bytes::Bytes::new(),
            }],
            total_records,
            total_bytes,
            ordered: false,
            app_metadata: bytes::Bytes::new(),
        };

        Ok(Response::new(info))
    }

    async fn poll_flight_info(
        &self,
        _request: Request<FlightDescriptor>,
    ) -> Result<Response<PollInfo>, Status> {
        Err(Status::unimplemented("PollFlightInfo not implemented"))
    }

    async fn get_schema(
        &self,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<SchemaResult>, Status> {
        let descriptor = request.into_inner();
        let desc = parse_get_descriptor(&descriptor).map_err(Status::from)?;

        let meta = self
            .engine
            .get_source(&desc.source_id)
            .ok_or_else(|| Status::not_found(format!("source '{}' not found", desc.source_id)))?;

        let schema = schema_bytes_from_source(&meta)
            .map_err(|e| Status::internal(format!("schema serialization failed: {e}")))?;

        Ok(Response::new(SchemaResult { schema }))
    }

    async fn do_get(
        &self,
        request: Request<Ticket>,
    ) -> Result<Response<Self::DoGetStream>, Status> {
        let ticket = request.into_inner();
        let fd = FlightDescriptor {
            r#type: 0,
            cmd: ticket.ticket,
            path: vec![],
        };
        let desc = parse_get_descriptor(&fd).map_err(Status::from)?;

        let view_expr = if let Some(bytes) = desc.view_expr_bytes.as_deref() {
            serde_json::from_slice::<tv_core::ViewExpr>(bytes)
                .map_err(|e| Status::invalid_argument(format!("invalid view_expr: {e}")))?
        } else {
            tv_core::ViewExpr {
                source_id: desc.source_id.clone(),
                ops: vec![],
            }
        };

        let engine = Arc::clone(&self.engine);
        let response = engine
            .query_view_tile(&view_expr, desc.row, desc.col, desc.rows, desc.cols)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let flight_data = ipc_bytes_to_flight_data(response.data)
            .map_err(|e| Status::internal(format!("IPC serialization failed: {e}")))?;

        let stream = futures::stream::iter(flight_data.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }

    async fn do_put(
        &self,
        request: Request<Streaming<FlightData>>,
    ) -> Result<Response<Self::DoPutStream>, Status> {
        let mut stream = request.into_inner();

        let first = stream
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("empty stream"))??;

        let descriptor = first
            .flight_descriptor
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("first message must have flight descriptor"))?;

        let put_desc = parse_put_descriptor(descriptor).map_err(Status::from)?;

        let mut all_data = vec![first];
        while let Some(item) = stream.next().await {
            all_data.push(item?);
        }

        let ipc_bytes = flight_data_to_ipc_bytes(all_data);

        let meta = self
            .engine
            .register_upload(bytes::Bytes::from(ipc_bytes), put_desc.name, false)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        info!(source_id = %meta.id, name = %meta.name, "Flight DoPut registered source");

        let app_metadata = serde_json::to_vec(&serde_json::json!({
            "source_id": meta.id,
            "n_rows": meta.n_rows,
            "n_cols": meta.n_cols,
        }))
        .map_err(|e| Status::internal(e.to_string()))?;

        let result = PutResult {
            app_metadata: bytes::Bytes::from(app_metadata),
        };

        let stream = futures::stream::once(async move { Ok(result) });
        Ok(Response::new(Box::pin(stream)))
    }

    async fn do_exchange(
        &self,
        _request: Request<Streaming<FlightData>>,
    ) -> Result<Response<Self::DoExchangeStream>, Status> {
        Err(Status::unimplemented("DoExchange not implemented"))
    }

    async fn do_action(
        &self,
        request: Request<Action>,
    ) -> Result<Response<Self::DoActionStream>, Status> {
        let action = request.into_inner();
        match action.r#type.as_str() {
            "list_sources" => {
                let sources = self.engine.list_sources();
                let json =
                    serde_json::to_vec(&sources).map_err(|e| Status::internal(e.to_string()))?;
                let result = arrow_flight::Result {
                    body: bytes::Bytes::from(json),
                };
                let stream = futures::stream::once(async move { Ok(result) });
                Ok(Response::new(Box::pin(stream)))
            }
            "delete_source" => {
                let source_id = String::from_utf8(action.body.to_vec())
                    .map_err(|e| Status::invalid_argument(e.to_string()))?;
                self.engine
                    .remove_source(&source_id)
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
                let stream = futures::stream::empty();
                Ok(Response::new(Box::pin(stream)))
            }
            other => {
                warn!(action = other, "unknown Flight action");
                Err(Status::unimplemented(format!(
                    "unknown action: '{other}'. Available: list_sources, delete_source"
                )))
            }
        }
    }

    async fn list_actions(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<Self::ListActionsStream>, Status> {
        let actions = vec![
            ActionType {
                r#type: "list_sources".to_string(),
                description: "List all registered sources".to_string(),
            },
            ActionType {
                r#type: "delete_source".to_string(),
                description: "Delete a source by ID".to_string(),
            },
        ];
        let stream = futures::stream::iter(actions.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }
}

fn source_to_flight_info(meta: &SourceMeta) -> Result<FlightInfo, Status> {
    let desc = GetDescriptor {
        source_id: meta.id.clone(),
        view_expr_bytes: None,
        row: 0,
        col: 0,
        rows: 256,
        cols: 64,
    };

    let ticket_bytes = encode_get_descriptor(&desc).map_err(|e| Status::internal(e.to_string()))?;

    let schema_bytes = schema_bytes_from_source(meta)
        .map_err(|e| Status::internal(format!("schema serialization failed: {e}")))?;

    Ok(FlightInfo {
        schema: schema_bytes,
        flight_descriptor: Some(FlightDescriptor {
            r#type: 1,
            cmd: bytes::Bytes::from(meta.id.clone()),
            path: vec![meta.name.clone()],
        }),
        endpoint: vec![arrow_flight::FlightEndpoint {
            ticket: Some(Ticket {
                ticket: ticket_bytes,
            }),
            location: vec![],
            expiration_time: None,
            app_metadata: bytes::Bytes::new(),
        }],
        total_records: meta.n_rows as i64,
        total_bytes: meta.file_size_bytes.min(i64::MAX as u64) as i64,
        ordered: false,
        app_metadata: bytes::Bytes::new(),
    })
}

fn schema_bytes_from_source(meta: &SourceMeta) -> Result<bytes::Bytes, ArrowError> {
    use arrow::datatypes::{Field, Schema};

    let fields: Vec<Field> = meta
        .columns
        .iter()
        .map(|col| Field::new(&col.name, parse_data_type(&col.data_type), col.nullable))
        .collect();

    let schema = Schema::new(fields);
    let gen = IpcDataGenerator::default();
    let mut dict_tracker = DictionaryTracker::new(false);
    let encoded = gen.schema_to_bytes_with_dictionary_tracker(
        &schema,
        &mut dict_tracker,
        &IpcWriteOptions::default(),
    );
    Ok(bytes::Bytes::from(encoded.ipc_message))
}

fn parse_data_type(type_str: &str) -> arrow::datatypes::DataType {
    use arrow::datatypes::DataType;
    match type_str {
        s if s.starts_with("Int8") => DataType::Int8,
        s if s.starts_with("Int16") => DataType::Int16,
        s if s.starts_with("Int32") => DataType::Int32,
        s if s.starts_with("Int64") => DataType::Int64,
        s if s.starts_with("UInt8") => DataType::UInt8,
        s if s.starts_with("UInt16") => DataType::UInt16,
        s if s.starts_with("UInt32") => DataType::UInt32,
        s if s.starts_with("UInt64") => DataType::UInt64,
        s if s.starts_with("Float32") => DataType::Float32,
        s if s.starts_with("Float64") => DataType::Float64,
        s if s.starts_with("Boolean") => DataType::Boolean,
        s if s.starts_with("Utf8") || s.starts_with("Str") => DataType::Utf8,
        s if s.starts_with("Binary") => DataType::Binary,
        s if s.starts_with("Date32") => DataType::Date32,
        s if s.starts_with("Date64") => DataType::Date64,
        _ => DataType::Utf8,
    }
}

fn ipc_bytes_to_flight_data(ipc: Vec<u8>) -> Result<Vec<FlightData>, ArrowError> {
    let cursor = std::io::Cursor::new(&ipc);
    let reader = StreamReader::try_new(cursor, None)?;

    let schema = reader.schema();
    let opts = IpcWriteOptions::default();
    let gen = IpcDataGenerator::default();
    let mut dict_tracker = DictionaryTracker::new(false);

    let schema_encoded =
        gen.schema_to_bytes_with_dictionary_tracker(&schema, &mut dict_tracker, &opts);
    let schema_flight = FlightData {
        flight_descriptor: None,
        data_header: bytes::Bytes::from(schema_encoded.ipc_message),
        app_metadata: bytes::Bytes::new(),
        data_body: bytes::Bytes::from(schema_encoded.arrow_data),
    };

    let mut result = vec![schema_flight];

    for batch in reader {
        let batch = batch?;
        let (_, encoded) = gen.encoded_batch(&batch, &mut dict_tracker, &opts)?;
        result.push(FlightData {
            flight_descriptor: None,
            data_header: bytes::Bytes::from(encoded.ipc_message),
            app_metadata: bytes::Bytes::new(),
            data_body: bytes::Bytes::from(encoded.arrow_data),
        });
    }

    Ok(result)
}

fn flight_data_to_ipc_bytes(data: Vec<FlightData>) -> Vec<u8> {
    let mut result = vec![];
    for fd in data {
        result.extend_from_slice(&fd.data_header);
        result.extend_from_slice(&fd.data_body);
    }
    result
}
