//! Parse gRPC method paths from `Grpc\BaseStub::_simpleRequest`.

/// Parsed `/Package.Service/method` path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcPath {
    pub service: String,
    pub method: String,
}

impl GrpcPath {
    /// Span name matching otelgrpc style: `{service}/{method}`.
    pub fn span_name(&self) -> String {
        format!("{}/{}", self.service, self.method)
    }
}

/// Parse a gRPC path like `/LinkServiceProto.LinkService/getTargetLink`.
pub fn parse_grpc_path(path: &str) -> Option<GrpcPath> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let path = path.strip_prefix('/').unwrap_or(path);
    let (service, method) = path.split_once('/')?;
    if service.is_empty() || method.is_empty() || method.contains('/') {
        return None;
    }
    Some(GrpcPath {
        service: service.to_string(),
        method: method.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_link_service_path() {
        let p = parse_grpc_path("/LinkServiceProto.LinkService/getTargetLink").unwrap();
        assert_eq!(p.service, "LinkServiceProto.LinkService");
        assert_eq!(p.method, "getTargetLink");
        assert_eq!(
            p.span_name(),
            "LinkServiceProto.LinkService/getTargetLink"
        );
    }

    #[test]
    fn parses_without_leading_slash() {
        let p = parse_grpc_path("CommonServiceProto.CommonService/getOsTypes").unwrap();
        assert_eq!(p.service, "CommonServiceProto.CommonService");
        assert_eq!(p.method, "getOsTypes");
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_grpc_path("").is_none());
        assert!(parse_grpc_path("/").is_none());
        assert!(parse_grpc_path("/OnlyService").is_none());
        assert!(parse_grpc_path("/Svc/").is_none());
        assert!(parse_grpc_path("/a/b/c").is_none());
    }
}
