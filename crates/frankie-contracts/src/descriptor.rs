use crate::error::ContractError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A schema given either by reference (the catalog manifests name a schema file) or inline (MCP `inputSchema`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SchemaRef {
    Ref(String),
    Inline(Value),
}

/// What a tool is. The five required fields are what every connector, including the two MCP servers
/// that emit only `name`, `description` and `inputSchema` today, can supply. The rest are optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub id: String,
    pub capability: String,
    pub description: String,
    #[serde(alias = "inputSchema")]
    pub input_schema: SchemaRef,
    pub mutates: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effects: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitations: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "outputSchema"
    )]
    pub output_schema: Option<SchemaRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_schema: Option<SchemaRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_version: Option<String>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// Read a catalog manifest list (`fixtures/catalog.manifests.json` shape) into descriptors.
/// Tool id is `{extension_id}.{module.id}.{operation.name}`. Any operation missing a required
/// field is an error, never skipped silently.
pub fn descriptors_from_catalog(catalog: &Value) -> Result<Vec<ToolDescriptor>, ContractError> {
    let list = catalog
        .as_array()
        .ok_or_else(|| ContractError::InvalidDescriptor("catalog must be a JSON array".into()))?;
    let mut out = Vec::new();
    for m in list {
        let ext = s(m, "extension_id")
            .ok_or_else(|| ContractError::InvalidDescriptor("missing extension_id".into()))?;
        let module = m
            .get("module")
            .ok_or_else(|| ContractError::InvalidDescriptor("missing module".into()))?;
        let mid = s(module, "id")
            .ok_or_else(|| ContractError::InvalidDescriptor("missing module.id".into()))?;
        let cv = module.get("contract").and_then(|c| s(c, "version"));
        let ops = module
            .get("operations")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ContractError::InvalidDescriptor(format!("{ext}.{mid}: missing operations"))
            })?;
        for op in ops {
            let name = s(op, "name").ok_or_else(|| {
                ContractError::InvalidDescriptor(format!("{ext}.{mid}: operation without name"))
            })?;
            let id = format!("{ext}.{mid}.{name}");
            let need = |k: &str| {
                s(op, k)
                    .ok_or_else(|| ContractError::InvalidDescriptor(format!("{id}: missing {k}")))
            };
            let schema = |k: &str| {
                op.get(k).map(|v| match v {
                    Value::String(r) => SchemaRef::Ref(r.clone()),
                    other => SchemaRef::Inline(other.clone()),
                })
            };
            out.push(ToolDescriptor {
                id: id.clone(),
                capability: need("capability")?,
                description: need("description")?,
                input_schema: schema("input_schema").ok_or_else(|| {
                    ContractError::InvalidDescriptor(format!("{id}: missing input_schema"))
                })?,
                mutates: op.get("mutates").and_then(Value::as_bool).ok_or_else(|| {
                    ContractError::InvalidDescriptor(format!("{id}: missing mutates"))
                })?,
                when_to_use: s(op, "when_to_use"),
                returns: s(op, "returns"),
                side_effects: s(op, "side_effects"),
                limitations: s(op, "limitations"),
                examples: op
                    .get("examples")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                output_schema: schema("output_schema"),
                error_schema: schema("error_schema"),
                contract_version: cv.clone(),
            });
        }
    }
    Ok(out)
}
