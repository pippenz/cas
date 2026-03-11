use crate::mcp::tools::service::imports::*;

impl CasService {
    pub(in crate::mcp::tools::service) async fn agent_register(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::AgentRegisterRequest;
        let inner_req = AgentRegisterRequest {
            name: req
                .name
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "name required"))?,
            agent_type: req.agent_type.unwrap_or_else(|| "primary".to_string()),
            session_id: req.session_id,
            parent_id: req.parent_id,
        };
        self.inner.cas_agent_register(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn agent_unregister(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::IdRequest;
        let inner_req = IdRequest {
            id: req
                .id
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "id required"))?,
        };
        self.inner.cas_agent_unregister(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn agent_whoami(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::IdRequest;
        let agent_id = match req.id {
            Some(id) => id,
            None => self.inner.get_agent_id()?,
        };
        let inner_req = IdRequest { id: agent_id };
        self.inner.cas_agent_whoami(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn agent_heartbeat(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::IdRequest;
        let agent_id = match req.id {
            Some(id) => id,
            None => self.inner.get_agent_id()?,
        };
        let inner_req = IdRequest { id: agent_id };
        self.inner.cas_agent_heartbeat(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn agent_list(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::LimitRequest;
        let inner_req = LimitRequest {
            limit: req.limit,
            scope: "all".to_string(),
            sort: None,
            sort_order: None,
            team_id: None,
        };
        self.inner.cas_agent_list(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn agent_cleanup(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::AgentCleanupRequest;
        let inner_req = AgentCleanupRequest {
            stale_threshold_secs: req.stale_threshold_secs,
        };
        self.inner.cas_agent_cleanup(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn agent_session_start(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::SessionStartRequest;
        let inner_req = SessionStartRequest {
            session_id: req.session_id,
            name: req.name,
            agent_type: req.agent_type,
            parent_id: req.parent_id,
            permission_mode: None,
            cwd: None,
            limit: req.limit,
        };
        self.inner
            .cas_agent_session_start(Parameters(inner_req))
            .await
    }

    pub(in crate::mcp::tools::service) async fn agent_session_end(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::SessionEndRequest;
        let session_id = match req.session_id.or(req.id) {
            Some(id) => Some(id),
            None => Some(self.inner.get_agent_id()?),
        };
        let inner_req = SessionEndRequest {
            session_id,
            reason: req.reason,
        };
        self.inner
            .cas_agent_session_end(Parameters(inner_req))
            .await
    }

    pub(in crate::mcp::tools::service) async fn loop_start(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::LoopStartRequest;
        let inner_req = LoopStartRequest {
            prompt: req
                .prompt
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "prompt required"))?,
            completion_promise: req.completion_promise,
            max_iterations: req.max_iterations.unwrap_or(0),
            task_id: req.task_id,
            session_id: req
                .session_id
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "session_id required"))?,
        };
        self.inner.cas_loop_start(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn loop_cancel(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::LoopCancelRequest;
        let inner_req = LoopCancelRequest {
            session_id: req
                .session_id
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "session_id required"))?,
            reason: req.reason,
        };
        self.inner.cas_loop_cancel(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn loop_status(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::LoopStatusRequest;
        let inner_req = LoopStatusRequest {
            session_id: req
                .session_id
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "session_id required"))?,
        };
        self.inner.cas_loop_status(Parameters(inner_req)).await
    }

    pub(in crate::mcp::tools::service) async fn lease_history(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::mcp::tools::LeaseHistoryRequest;
        let inner_req = LeaseHistoryRequest {
            task_id: req
                .task_id
                .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "task_id required"))?,
            limit: req.limit,
        };
        self.inner.cas_lease_history(Parameters(inner_req)).await
    }
}
