use crate::support::*;
use cas::mcp::tools::*;
use rmcp::handler::server::wrapper::Parameters;

#[tokio::test]
async fn test_remember_basic() {
    let (_temp, service) = setup_cas();

    let req = RememberRequest {
        scope: "project".to_string(),
        content: "Test memory content".to_string(),
        entry_type: "learning".to_string(),
        tags: Some("test,memory".to_string()),
        title: Some("Test Title".to_string()),
        importance: 0.7,
        valid_from: None,
        valid_until: None,
        team_id: None,
    };

    let result = service
        .cas_remember(Parameters(req))
        .await
        .expect("remember should succeed");

    let text = extract_text(result);
    assert!(text.contains("Created entry"));
    assert!(extract_entry_id(&text).is_some(), "Should contain entry ID");
}

#[tokio::test]
async fn test_remember_with_defaults() {
    let (_temp, service) = setup_cas();

    let req = RememberRequest {
        scope: "project".to_string(),
        content: "Simple memory".to_string(),
        entry_type: "learning".to_string(),
        tags: None,
        title: None,
        importance: 0.5,
        valid_from: None,
        valid_until: None,
        team_id: None,
    };

    let result = service
        .cas_remember(Parameters(req))
        .await
        .expect("remember should succeed");

    let text = extract_text(result);
    assert!(text.contains("Created entry"));
}

#[tokio::test]
async fn test_get_entry() {
    let (_temp, service) = setup_cas();

    // First create an entry
    let req = RememberRequest {
        scope: "project".to_string(),
        content: "Test get content".to_string(),
        entry_type: "learning".to_string(),
        tags: None,
        title: None,
        importance: 0.5,
        valid_from: None,
        valid_until: None,
        team_id: None,
    };

    let result = service
        .cas_remember(Parameters(req))
        .await
        .expect("remember should succeed");

    let text = extract_text(result);
    let id = extract_entry_id(&text).expect("should have ID");

    // Now get the entry
    let get_req = IdRequest { id: id.to_string() };
    let result = service
        .cas_get(Parameters(get_req))
        .await
        .expect("get should succeed");

    let text = extract_text(result);
    assert!(text.contains("Test get content"));
    assert!(text.contains("Learning"));
}

#[tokio::test]
async fn test_get_nonexistent_entry() {
    let (_temp, service) = setup_cas();

    let req = IdRequest {
        id: "nonexistent-id".to_string(),
    };

    let result = service.cas_get(Parameters(req)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_update_entry() {
    let (_temp, service) = setup_cas();

    // Create entry
    let req = RememberRequest {
        scope: "project".to_string(),
        content: "Original content".to_string(),
        entry_type: "learning".to_string(),
        tags: None,
        title: None,
        importance: 0.5,
        valid_from: None,
        valid_until: None,
        team_id: None,
    };

    let result = service
        .cas_remember(Parameters(req))
        .await
        .expect("remember should succeed");

    let text = extract_text(result);
    let id = extract_entry_id(&text).expect("should have ID");

    // Update entry
    let update_req = EntryUpdateRequest {
        id: id.to_string(),
        content: Some("Updated content".to_string()),
        tags: Some("updated,test".to_string()),
        importance: Some(0.9),
    };

    let result = service
        .cas_update(Parameters(update_req))
        .await
        .expect("update should succeed");

    let text = extract_text(result);
    assert!(text.contains("Updated"));
    assert!(text.contains("content"));

    // Verify update
    let get_req = IdRequest { id: id.to_string() };
    let result = service
        .cas_get(Parameters(get_req))
        .await
        .expect("get should succeed");

    let text = extract_text(result);
    assert!(text.contains("Updated content"));
}

#[tokio::test]
async fn test_archive_and_unarchive() {
    let (_temp, service) = setup_cas();

    // Create entry
    let req = RememberRequest {
        scope: "project".to_string(),
        content: "Archive test".to_string(),
        entry_type: "learning".to_string(),
        tags: None,
        title: None,
        importance: 0.5,
        valid_from: None,
        valid_until: None,
        team_id: None,
    };

    let result = service
        .cas_remember(Parameters(req))
        .await
        .expect("remember should succeed");

    let text = extract_text(result);
    let id = extract_entry_id(&text).expect("should have ID");

    // Archive
    let archive_req = IdRequest { id: id.to_string() };
    let result = service
        .cas_archive(Parameters(archive_req))
        .await
        .expect("archive should succeed");

    let text = extract_text(result);
    assert!(text.contains("Archived"));

    // Unarchive
    let unarchive_req = IdRequest { id: id.to_string() };
    let result = service
        .cas_unarchive(Parameters(unarchive_req))
        .await
        .expect("unarchive should succeed");

    let text = extract_text(result);
    assert!(text.contains("Restored"));
}

#[tokio::test]
async fn test_helpful_and_harmful() {
    let (_temp, service) = setup_cas();

    // Create entry
    let req = RememberRequest {
        scope: "project".to_string(),
        content: "Feedback test".to_string(),
        entry_type: "learning".to_string(),
        tags: None,
        title: None,
        importance: 0.5,
        valid_from: None,
        valid_until: None,
        team_id: None,
    };

    let result = service
        .cas_remember(Parameters(req))
        .await
        .expect("remember should succeed");

    let text = extract_text(result);
    let id = extract_entry_id(&text).expect("should have ID");

    // Mark helpful
    let helpful_req = IdRequest { id: id.to_string() };
    let result = service
        .cas_helpful(Parameters(helpful_req))
        .await
        .expect("helpful should succeed");

    let text = extract_text(result);
    assert!(text.contains("helpful"));

    // Mark harmful
    let harmful_req = IdRequest { id: id.to_string() };
    let result = service
        .cas_harmful(Parameters(harmful_req))
        .await
        .expect("harmful should succeed");

    let text = extract_text(result);
    assert!(text.contains("harmful"));
}

#[tokio::test]
async fn test_list_entries() {
    let (_temp, service) = setup_cas();

    // Create a few entries
    for i in 0..3 {
        let req = RememberRequest {
            scope: "project".to_string(),
            content: format!("List test entry {i}"),
            entry_type: "learning".to_string(),
            tags: None,
            title: None,
            importance: 0.5,
            valid_from: None,
            valid_until: None,
            team_id: None,
        };
        service
            .cas_remember(Parameters(req))
            .await
            .expect("remember should succeed");
    }

    // List entries
    let list_req = LimitRequest {
        scope: "all".to_string(),
        limit: Some(10),
        sort: None,
        sort_order: None,
        team_id: None,
    };
    let result = service
        .cas_list(Parameters(list_req))
        .await
        .expect("list should succeed");

    let text = extract_text(result);
    assert!(text.contains("Entries"));
    assert!(text.contains("List test entry"));
}

#[tokio::test]
async fn test_recent_entries() {
    let (_temp, service) = setup_cas();

    // Create entries
    for i in 0..3 {
        let req = RememberRequest {
            scope: "project".to_string(),
            content: format!("Recent test entry {i}"),
            entry_type: "learning".to_string(),
            tags: None,
            title: None,
            importance: 0.5,
            valid_from: None,
            valid_until: None,
            team_id: None,
        };
        service
            .cas_remember(Parameters(req))
            .await
            .expect("remember should succeed");
    }

    // Get recent
    let recent_req = RecentRequest { n: 5 };
    let result = service
        .cas_recent(Parameters(recent_req))
        .await
        .expect("recent should succeed");

    let text = extract_text(result);
    assert!(text.contains("Recent entries"));
}

#[tokio::test]
async fn test_delete_entry() {
    let (_temp, service) = setup_cas();

    // Create entry
    let req = RememberRequest {
        scope: "project".to_string(),
        content: "Delete test".to_string(),
        entry_type: "learning".to_string(),
        tags: None,
        title: None,
        importance: 0.5,
        valid_from: None,
        valid_until: None,
        team_id: None,
    };

    let result = service
        .cas_remember(Parameters(req))
        .await
        .expect("remember should succeed");

    let text = extract_text(result);
    let id = extract_entry_id(&text).expect("should have ID");

    // Delete
    let delete_req = IdRequest { id: id.to_string() };
    let result = service
        .cas_delete(Parameters(delete_req))
        .await
        .expect("delete should succeed");

    let text = extract_text(result);
    assert!(text.contains("Deleted"));

    // Verify deleted
    let get_req = IdRequest { id: id.to_string() };
    let result = service.cas_get(Parameters(get_req)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_set_tier() {
    let (_temp, service) = setup_cas();

    // Create entry
    let req = RememberRequest {
        scope: "project".to_string(),
        content: "Tier test".to_string(),
        entry_type: "learning".to_string(),
        tags: None,
        title: None,
        importance: 0.5,
        valid_from: None,
        valid_until: None,
        team_id: None,
    };

    let result = service
        .cas_remember(Parameters(req))
        .await
        .expect("remember should succeed");

    let text = extract_text(result);
    let id = extract_entry_id(&text).expect("should have ID");

    // Set tier to cold
    let tier_req = MemoryTierRequest {
        id: id.to_string(),
        tier: "cold".to_string(),
    };
    let result = service
        .cas_set_tier(Parameters(tier_req))
        .await
        .expect("set_tier should succeed");

    let text = extract_text(result);
    assert!(text.contains("cold") || text.contains("tier"));
}
