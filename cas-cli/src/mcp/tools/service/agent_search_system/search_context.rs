use crate::mcp::tools::service::imports::*;

impl CasService {
    pub(in crate::mcp::tools::service) async fn search_impl(
        &self,
        req: SearchContextRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::SearchRequest;
        let inner_req = SearchRequest {
            query: req
                .query
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "query required"))?,
            limit: req.limit.unwrap_or(10),
            doc_type: req.doc_type,
            scope: req.scope.unwrap_or_else(|| "all".to_string()),
            tags: req.tags,
        };
        self.inner.cas_search(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn context_impl(
        &self,
        req: SearchContextRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::LimitRequest;
        let inner_req = LimitRequest {
            limit: req.limit,
            scope: req.scope.unwrap_or_else(|| "all".to_string()),
            sort: None,
            sort_order: None,
            team_id: None,
        };
        self.inner.cas_context(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn context_for_subagent_impl(
        &self,
        req: SearchContextRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::SubAgentContextRequest;
        let inner_req = SubAgentContextRequest {
            task_id: req
                .task_id
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "task_id required"))?,
            max_tokens: req.max_tokens.unwrap_or(2000),
            include_memories: req.include_memories.unwrap_or(true),
        };
        self.inner
            .cas_context_for_subagent(Parameters(inner_req))
            .await
    }

    pub(in crate::mcp::tools::service) async fn observe_impl(
        &self,
        req: SearchContextRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::ObserveRequest;
        let inner_req = ObserveRequest {
            content: req
                .content
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "content required"))?,
            observation_type: req
                .observation_type
                .unwrap_or_else(|| "general".to_string()),
            source_tool: req.source_tool,
            tags: req.tags,
            scope: req.scope.unwrap_or_else(|| "project".to_string()),
        };
        self.inner.cas_observe(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn entity_list_impl(
        &self,
        req: SearchContextRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::EntityListRequest;
        let inner_req = EntityListRequest {
            entity_type: req.entity_type.clone(),
            query: req.query.clone(),
            tags: req.tags.clone(),
            scope: req.scope.clone(),
            sort: req.sort,
            sort_order: req.sort_order,
            limit: req.limit,
        };
        self.inner.cas_entity_list(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn entity_show_impl(
        &self,
        req: SearchContextRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::IdRequest;
        let inner_req = IdRequest {
            id: req
                .id
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "id required"))?,
        };
        self.inner.cas_entity_show(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn entity_extract_impl(
        &self,
        req: SearchContextRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::EntityExtractRequest;
        let inner_req = EntityExtractRequest {
            query: req.query,
            scope: req.scope,
            tags: req.tags,
            entity_type: req.entity_type,
            limit: req.limit,
        };
        self.inner.cas_entity_extract(Parameters(inner_req)).await
    }
}
