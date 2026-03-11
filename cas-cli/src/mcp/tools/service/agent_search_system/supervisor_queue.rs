use crate::mcp::tools::service::imports::*;

impl CasService {
    pub(in crate::mcp::tools::service) async fn queue_notify(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{NotificationPriority, open_supervisor_queue_store};

        let supervisor_id = req.supervisor_id.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "supervisor_id required for queue_notify",
            )
        })?;
        let event_type = req.event_type.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "event_type required for queue_notify",
            )
        })?;
        let payload = req.payload.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "payload required for queue_notify",
            )
        })?;

        let priority = match req.priority.as_deref() {
            Some("critical") | Some("0") => NotificationPriority::Critical,
            Some("high") | Some("1") => NotificationPriority::High,
            _ => NotificationPriority::Normal,
        };

        let queue = open_supervisor_queue_store(&self.inner.cas_root).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open queue: {error}"),
            )
        })?;

        let notification_id = queue
            .notify(&supervisor_id, &event_type, &payload, priority)
            .map_err(|error| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to queue notification: {error}"),
                )
            })?;

        Ok(Self::success(format!(
            "Notification queued successfully\n\nID: {notification_id}\nSupervisor: {supervisor_id}\nType: {event_type}\nPriority: {priority:?}"
        )))
    }

    pub(in crate::mcp::tools::service) async fn queue_poll(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_supervisor_queue_store;

        let supervisor_id = req
            .supervisor_id
            .or_else(|| self.inner.get_agent_id().ok())
            .ok_or_else(|| {
                Self::error(
                    ErrorCode::INVALID_PARAMS,
                    "supervisor_id required for queue_poll (or register as an agent first)",
                )
            })?;
        let limit = req.limit.unwrap_or(10);

        let queue = open_supervisor_queue_store(&self.inner.cas_root).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open queue: {error}"),
            )
        })?;

        let notifications = queue.poll(&supervisor_id, limit).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to poll queue: {error}"),
            )
        })?;

        if notifications.is_empty() {
            return Ok(Self::success("No pending notifications"));
        }

        let mut output = format!(
            "Polled {} notification(s) (marked as processed):\n\n",
            notifications.len()
        );
        for notification in &notifications {
            output.push_str(&format!(
                "**[{}]** {} - {:?}\n  Payload: {}\n  Created: {}\n\n",
                notification.id,
                notification.event_type,
                notification.priority,
                notification.payload,
                notification.created_at.format("%H:%M:%S")
            ));
        }

        Ok(Self::success(output))
    }

    pub(in crate::mcp::tools::service) async fn queue_peek(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_supervisor_queue_store;

        let supervisor_id = req
            .supervisor_id
            .or_else(|| self.inner.get_agent_id().ok())
            .ok_or_else(|| {
                Self::error(
                    ErrorCode::INVALID_PARAMS,
                    "supervisor_id required for queue_peek (or register as an agent first)",
                )
            })?;
        let limit = req.limit.unwrap_or(10);

        let queue = open_supervisor_queue_store(&self.inner.cas_root).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open queue: {error}"),
            )
        })?;

        let notifications = queue.peek(&supervisor_id, limit).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to peek queue: {error}"),
            )
        })?;

        if notifications.is_empty() {
            return Ok(Self::success("No pending notifications"));
        }

        let mut output = format!(
            "Peeked {} pending notification(s):\n\n",
            notifications.len()
        );
        for notification in &notifications {
            output.push_str(&format!(
                "**[{}]** {} - {:?}\n  Payload: {}\n  Created: {}\n\n",
                notification.id,
                notification.event_type,
                notification.priority,
                notification.payload,
                notification.created_at.format("%H:%M:%S")
            ));
        }
        output.push_str(
            "Use `queue_poll` to process or `queue_ack` to acknowledge individual notifications.",
        );

        Ok(Self::success(output))
    }

    pub(in crate::mcp::tools::service) async fn queue_ack(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_supervisor_queue_store;

        let notification_id = req.notification_id.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "notification_id required for queue_ack",
            )
        })?;

        let queue = open_supervisor_queue_store(&self.inner.cas_root).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open queue: {error}"),
            )
        })?;

        queue.ack(notification_id).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to acknowledge: {error}"),
            )
        })?;

        Ok(Self::success(format!(
            "Notification {notification_id} acknowledged"
        )))
    }
}
