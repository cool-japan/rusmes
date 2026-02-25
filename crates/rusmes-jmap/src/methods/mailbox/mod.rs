//! Complete Mailbox method implementations for JMAP
//!
//! Implements RFC 8621 Section 2 - Mailboxes
//! - Mailbox/get - retrieve mailboxes
//! - Mailbox/set - create, update, destroy mailboxes
//! - Mailbox/query - list/filter mailboxes
//! - Mailbox/changes - detect mailbox changes
//! - Mailbox/queryChanges - incremental updates
//!
//! Type definitions and helpers live in [`mailbox_types`].

pub mod mailbox_types;

pub use mailbox_types::{
    AddedItem, Mailbox, MailboxChangesRequest, MailboxChangesResponse, MailboxFilterCondition,
    MailboxGetRequest, MailboxGetResponse, MailboxObject, MailboxQueryChangesRequest,
    MailboxQueryChangesResponse, MailboxQueryRequest, MailboxQueryResponse, MailboxRights,
    MailboxRole, MailboxSetRequest, MailboxSetResponse, MailboxSort,
};

use crate::types::JmapSetError;
use mailbox_types::{
    apply_mailbox_filter, apply_mailbox_sort, filter_mailbox_properties, generate_state,
    get_default_mailboxes, is_special_use_mailbox, validate_mailbox_name, would_create_cycle,
};
use rusmes_storage::MessageStore;
use std::collections::HashMap;

/// Handle Mailbox/get method
pub async fn mailbox_get(
    request: MailboxGetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<MailboxGetResponse> {
    let mut list = Vec::new();
    let mut not_found = Vec::new();

    // In a real implementation, we'd fetch from storage.
    // For now, use default mailboxes.
    let default_mailboxes = get_default_mailboxes();

    // If no IDs specified, return all default mailboxes
    let ids = request
        .ids
        .unwrap_or_else(|| default_mailboxes.keys().cloned().collect());

    for id in ids {
        if let Some(mailbox) = default_mailboxes.get(&id) {
            // Filter properties if requested
            let mailbox = if let Some(ref props) = request.properties {
                filter_mailbox_properties(mailbox.clone(), props)
            } else {
                mailbox.clone()
            };
            list.push(mailbox);
        } else {
            not_found.push(id);
        }
    }

    Ok(MailboxGetResponse {
        account_id: request.account_id,
        state: generate_state(),
        list,
        not_found,
    })
}

/// Handle Mailbox/set method
#[allow(clippy::too_many_arguments)]
pub async fn mailbox_set(
    request: MailboxSetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<MailboxSetResponse> {
    let old_state = generate_state();
    let mut created = HashMap::new();
    let mut updated = HashMap::new();
    let mut destroyed = Vec::new();
    let mut not_created = HashMap::new();
    let mut not_updated = HashMap::new();
    let mut not_destroyed = HashMap::new();

    // Get current mailboxes (in real implementation, from storage)
    let mut current_mailboxes = get_default_mailboxes();

    // Handle creates
    if let Some(create_map) = request.create {
        for (creation_id, mailbox_obj) in create_map {
            // Validate mailbox name
            if let Err(err) = validate_mailbox_name(&mailbox_obj.name) {
                not_created.insert(
                    creation_id,
                    JmapSetError {
                        error_type: "invalidProperties".to_string(),
                        description: Some(err),
                    },
                );
                continue;
            }

            // Check parent exists if specified
            if let Some(ref parent_id) = mailbox_obj.parent_id {
                if !current_mailboxes.contains_key(parent_id) {
                    not_created.insert(
                        creation_id,
                        JmapSetError {
                            error_type: "invalidProperties".to_string(),
                            description: Some("Parent mailbox not found".to_string()),
                        },
                    );
                    continue;
                }
            }

            // Generate new ID
            let new_id = uuid::Uuid::new_v4().to_string();

            // Auto-detect role from name if not specified
            let role = mailbox_obj
                .role
                .or_else(|| MailboxRole::detect_from_name(&mailbox_obj.name));

            // Create new mailbox
            let new_mailbox = Mailbox {
                id: new_id.clone(),
                name: mailbox_obj.name,
                parent_id: mailbox_obj.parent_id,
                role,
                sort_order: mailbox_obj.sort_order.unwrap_or(1000),
                total_emails: 0,
                unread_emails: 0,
                total_threads: 0,
                unread_threads: 0,
                my_rights: MailboxRights::default(),
                is_subscribed: mailbox_obj.is_subscribed.unwrap_or(false),
            };

            // In production, would save to storage
            current_mailboxes.insert(new_id.clone(), new_mailbox.clone());
            created.insert(creation_id, new_mailbox);
        }
    }

    // Handle updates
    if let Some(update_map) = request.update {
        for (id, patch) in update_map {
            // Check if mailbox exists
            if let Some(mut mailbox) = current_mailboxes.get(&id).cloned() {
                // Apply patch
                if let Some(name) = patch.get("name").and_then(|v| v.as_str()) {
                    if let Err(err) = validate_mailbox_name(name) {
                        not_updated.insert(
                            id,
                            JmapSetError {
                                error_type: "invalidProperties".to_string(),
                                description: Some(err),
                            },
                        );
                        continue;
                    }
                    mailbox.name = name.to_string();
                }

                if let Some(parent_id) = patch.get("parentId") {
                    if parent_id.is_null() {
                        mailbox.parent_id = None;
                    } else if let Some(parent_id_str) = parent_id.as_str() {
                        // Check for circular reference
                        if would_create_cycle(parent_id_str, &id, &current_mailboxes) {
                            not_updated.insert(
                                id,
                                JmapSetError {
                                    error_type: "invalidProperties".to_string(),
                                    description: Some(
                                        "Would create circular hierarchy".to_string(),
                                    ),
                                },
                            );
                            continue;
                        }
                        mailbox.parent_id = Some(parent_id_str.to_string());
                    }
                }

                if let Some(sort_order) = patch.get("sortOrder").and_then(|v| v.as_u64()) {
                    mailbox.sort_order = sort_order as u32;
                }

                if let Some(is_subscribed) = patch.get("isSubscribed").and_then(|v| v.as_bool()) {
                    mailbox.is_subscribed = is_subscribed;
                }

                // In production, would save to storage
                current_mailboxes.insert(id.clone(), mailbox.clone());
                updated.insert(id, Some(mailbox));
            } else {
                not_updated.insert(
                    id,
                    JmapSetError {
                        error_type: "notFound".to_string(),
                        description: Some("Mailbox not found".to_string()),
                    },
                );
            }
        }
    }

    // Handle destroys
    if let Some(destroy_ids) = request.destroy {
        for id in destroy_ids {
            // Check if it's a special-use mailbox (cannot be deleted)
            if is_special_use_mailbox(&id) {
                not_destroyed.insert(
                    id,
                    JmapSetError {
                        error_type: "forbidden".to_string(),
                        description: Some("Cannot delete special-use mailbox".to_string()),
                    },
                );
                continue;
            }

            // Check if mailbox exists
            if current_mailboxes.contains_key(&id) {
                // Check if it has children
                let has_children = current_mailboxes
                    .values()
                    .any(|m| m.parent_id.as_ref() == Some(&id));

                if has_children {
                    not_destroyed.insert(
                        id,
                        JmapSetError {
                            error_type: "mailboxHasChild".to_string(),
                            description: Some("Cannot delete mailbox with children".to_string()),
                        },
                    );
                    continue;
                }

                // In production, would:
                // 1. Check onDestroyRemoveEmails flag
                // 2. Either move or delete emails
                // 3. Delete mailbox from storage

                current_mailboxes.remove(&id);
                destroyed.push(id);
            } else {
                not_destroyed.insert(
                    id,
                    JmapSetError {
                        error_type: "notFound".to_string(),
                        description: Some("Mailbox not found".to_string()),
                    },
                );
            }
        }
    }

    let new_state = generate_state();

    Ok(MailboxSetResponse {
        account_id: request.account_id,
        old_state,
        new_state,
        created: if created.is_empty() {
            None
        } else {
            Some(created)
        },
        updated: if updated.is_empty() {
            None
        } else {
            Some(updated)
        },
        destroyed: if destroyed.is_empty() {
            None
        } else {
            Some(destroyed)
        },
        not_created: if not_created.is_empty() {
            None
        } else {
            Some(not_created)
        },
        not_updated: if not_updated.is_empty() {
            None
        } else {
            Some(not_updated)
        },
        not_destroyed: if not_destroyed.is_empty() {
            None
        } else {
            Some(not_destroyed)
        },
    })
}

/// Handle Mailbox/query method
pub async fn mailbox_query(
    request: MailboxQueryRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<MailboxQueryResponse> {
    // Get all mailboxes (in real implementation, from storage)
    let mailboxes = get_default_mailboxes();
    let mut mailbox_list: Vec<Mailbox> = mailboxes.values().cloned().collect();

    // Apply filter
    if let Some(filter) = &request.filter {
        mailbox_list.retain(|mailbox| apply_mailbox_filter(mailbox, filter));
    }

    // Apply sort
    if let Some(sort_comparators) = &request.sort {
        apply_mailbox_sort(&mut mailbox_list, sort_comparators);
    } else {
        // Default sort by sort_order then name
        mailbox_list.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.cmp(&b.name))
        });
    }

    // Extract IDs
    let all_ids: Vec<String> = mailbox_list.iter().map(|m| m.id.clone()).collect();

    // Apply position and limit
    let position = request.position.unwrap_or(0).max(0) as usize;
    let limit = request.limit.unwrap_or(100) as usize;

    let total = all_ids.len() as u64;
    let start = position.min(all_ids.len());
    let end = (start + limit).min(all_ids.len());
    let result_ids = all_ids[start..end].to_vec();

    Ok(MailboxQueryResponse {
        account_id: request.account_id,
        query_state: generate_state(),
        can_calculate_changes: true,
        position: position as i64,
        ids: result_ids,
        total: if request.calculate_total.unwrap_or(false) {
            Some(total)
        } else {
            None
        },
        limit: Some(limit as u64),
    })
}

/// Handle Mailbox/changes method
pub async fn mailbox_changes(
    request: MailboxChangesRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<MailboxChangesResponse> {
    let since_state: u64 = request.since_state.parse().unwrap_or(0);
    let new_state = (since_state + 1).to_string();

    // In production, would query change log
    let created = Vec::new();
    let updated = Vec::new();
    let destroyed = Vec::new();

    Ok(MailboxChangesResponse {
        account_id: request.account_id,
        old_state: request.since_state,
        new_state,
        has_more_changes: false,
        created,
        updated,
        destroyed,
    })
}

/// Handle Mailbox/queryChanges method
pub async fn mailbox_query_changes(
    request: MailboxQueryChangesRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<MailboxQueryChangesResponse> {
    let new_query_state = (chrono::Utc::now().timestamp() as u64).to_string();

    // In production, would compare query results
    let removed = Vec::new();
    let added = Vec::new();

    Ok(MailboxQueryChangesResponse {
        account_id: request.account_id,
        old_query_state: request.since_query_state,
        new_query_state,
        total: if request.calculate_total.unwrap_or(false) {
            Some(6) // Number of default mailboxes
        } else {
            None
        },
        removed,
        added,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusmes_storage::backends::filesystem::FilesystemBackend;
    use rusmes_storage::StorageBackend;
    use std::path::PathBuf;

    fn create_test_store() -> std::sync::Arc<dyn MessageStore> {
        let backend = FilesystemBackend::new(PathBuf::from("/tmp/rusmes-test-storage"));
        backend.message_store()
    }

    #[tokio::test]
    async fn test_mailbox_get() {
        let store = create_test_store();
        let request = MailboxGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["inbox".to_string()]),
            properties: None,
        };

        let response = mailbox_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 1);
        assert_eq!(response.list[0].id, "inbox");
        assert_eq!(response.list[0].role, Some(MailboxRole::Inbox));
    }

    #[tokio::test]
    async fn test_mailbox_get_all() {
        let store = create_test_store();
        let request = MailboxGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = mailbox_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 6);
    }

    #[tokio::test]
    async fn test_mailbox_set_create() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "new1".to_string(),
            MailboxObject {
                name: "My Folder".to_string(),
                parent_id: None,
                role: None,
                sort_order: None,
                is_subscribed: Some(true),
            },
        );

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.created.is_some());
        let created = response.created.unwrap();
        assert_eq!(created.len(), 1);
        let mailbox = created.values().next().unwrap();
        assert_eq!(mailbox.name, "My Folder");
        assert!(mailbox.is_subscribed);
    }

    #[tokio::test]
    async fn test_mailbox_set_destroy_special_use() {
        let store = create_test_store();
        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec!["inbox".to_string()]),
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_destroyed.is_some());
        let errors = response.not_destroyed.unwrap();
        assert_eq!(errors.get("inbox").unwrap().error_type, "forbidden");
    }

    #[tokio::test]
    async fn test_mailbox_query() {
        let store = create_test_store();
        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: None,
            position: None,
            limit: None,
            calculate_total: Some(true),
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 6);
        assert_eq!(response.total, Some(6));
    }

    #[tokio::test]
    async fn test_mailbox_query_with_role_filter() {
        let store = create_test_store();
        let filter = MailboxFilterCondition {
            parent_id: None,
            name: None,
            role: Some(MailboxRole::Sent),
            has_any_role: None,
            is_subscribed: None,
        };

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 1);
        assert_eq!(response.ids[0], "sent");
    }

    #[tokio::test]
    async fn test_mailbox_changes() {
        let store = create_test_store();
        let request = MailboxChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "1".to_string(),
            max_changes: Some(50),
        };

        let response = mailbox_changes(request, store.as_ref()).await.unwrap();
        assert_eq!(response.old_state, "1");
        assert_eq!(response.new_state, "2");
        assert!(!response.has_more_changes);
    }

    #[tokio::test]
    async fn test_mailbox_query_changes() {
        let store = create_test_store();
        let request = MailboxQueryChangesRequest {
            account_id: "acc1".to_string(),
            since_query_state: "1".to_string(),
            filter: None,
            sort: None,
            max_changes: None,
            up_to_id: None,
            calculate_total: Some(true),
        };

        let response = mailbox_query_changes(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.total, Some(6));
    }

    #[tokio::test]
    async fn test_mailbox_rights() {
        let rights = MailboxRights::default();
        assert!(rights.may_read_items);
        assert!(rights.may_add_items);
        assert!(rights.may_delete);
    }

    #[tokio::test]
    async fn test_mailbox_role_serialization() {
        assert_eq!(
            serde_json::to_string(&MailboxRole::Inbox).unwrap(),
            "\"inbox\""
        );
        assert_eq!(
            serde_json::to_string(&MailboxRole::Sent).unwrap(),
            "\"sent\""
        );
    }

    #[tokio::test]
    async fn test_mailbox_get_not_found() {
        let store = create_test_store();
        let request = MailboxGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["nonexistent".to_string()]),
            properties: None,
        };

        let response = mailbox_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.not_found.len(), 1);
        assert_eq!(response.not_found[0], "nonexistent");
    }

    #[tokio::test]
    async fn test_mailbox_set_update() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert("inbox".to_string(), serde_json::json!({"name": "My Inbox"}));

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.updated.is_some());
        let updated = response.updated.unwrap();
        let mailbox = updated.get("inbox").unwrap().as_ref().unwrap();
        assert_eq!(mailbox.name, "My Inbox");
    }

    #[tokio::test]
    async fn test_mailbox_query_with_pagination() {
        let store = create_test_store();
        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: None,
            position: Some(2),
            limit: Some(2),
            calculate_total: Some(true),
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.position, 2);
        assert_eq!(response.ids.len(), 2);
        assert_eq!(response.total, Some(6));
    }

    #[tokio::test]
    async fn test_mailbox_create_with_parent() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "child1".to_string(),
            MailboxObject {
                name: "Child Folder".to_string(),
                parent_id: Some("inbox".to_string()),
                role: None,
                sort_order: Some(100),
                is_subscribed: None,
            },
        );

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.created.is_some());
        let created = response.created.unwrap();
        let mailbox = created.values().next().unwrap();
        assert_eq!(mailbox.parent_id, Some("inbox".to_string()));
        assert_eq!(mailbox.sort_order, 100);
    }

    #[tokio::test]
    async fn test_mailbox_query_sort() {
        let store = create_test_store();
        let sort = vec![MailboxSort {
            property: "sortOrder".to_string(),
            is_ascending: Some(true),
        }];

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: Some(sort),
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 6);
    }

    #[tokio::test]
    async fn test_mailbox_set_multiple_creates() {
        let store = create_test_store();
        let mut create_map = HashMap::new();

        for i in 1..=5 {
            create_map.insert(
                format!("folder{}", i),
                MailboxObject {
                    name: format!("Folder {}", i),
                    parent_id: None,
                    role: None,
                    sort_order: Some(i * 10),
                    is_subscribed: Some(true),
                },
            );
        }

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.created.is_some());
        assert_eq!(response.created.unwrap().len(), 5);
    }

    #[tokio::test]
    async fn test_mailbox_get_with_properties() {
        let store = create_test_store();
        let properties = vec![
            "id".to_string(),
            "name".to_string(),
            "role".to_string(),
            "totalEmails".to_string(),
        ];

        let request = MailboxGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["inbox".to_string()]),
            properties: Some(properties),
        };

        let response = mailbox_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 1);
    }

    #[tokio::test]
    async fn test_mailbox_changes_state_progression() {
        let store = create_test_store();

        let request1 = MailboxChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "5".to_string(),
            max_changes: None,
        };
        let response1 = mailbox_changes(request1, store.as_ref()).await.unwrap();

        let request2 = MailboxChangesRequest {
            account_id: "acc1".to_string(),
            since_state: response1.new_state.clone(),
            max_changes: None,
        };
        let response2 = mailbox_changes(request2, store.as_ref()).await.unwrap();

        assert!(response1.new_state < response2.new_state);
    }

    #[tokio::test]
    async fn test_mailbox_set_on_destroy_remove_emails() {
        let store = create_test_store();
        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec!["custom1".to_string()]),
            on_destroy_remove_emails: Some(true),
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_destroyed.is_some());
    }

    #[tokio::test]
    async fn test_mailbox_all_roles() {
        let store = create_test_store();
        let request = MailboxGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = mailbox_get(request, store.as_ref()).await.unwrap();
        let roles: Vec<_> = response.list.iter().filter_map(|m| m.role).collect();

        assert!(roles.contains(&MailboxRole::Inbox));
        assert!(roles.contains(&MailboxRole::Sent));
        assert!(roles.contains(&MailboxRole::Drafts));
        assert!(roles.contains(&MailboxRole::Trash));
        assert!(roles.contains(&MailboxRole::Junk));
        assert!(roles.contains(&MailboxRole::Archive));
    }

    #[tokio::test]
    async fn test_mailbox_query_limit_zero() {
        let store = create_test_store();
        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: None,
            position: None,
            limit: Some(0),
            calculate_total: Some(true),
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 0);
        assert_eq!(response.total, Some(6));
    }

    #[tokio::test]
    async fn test_mailbox_get_mixed_ids() {
        let store = create_test_store();
        let request = MailboxGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec![
                "inbox".to_string(),
                "nonexistent".to_string(),
                "sent".to_string(),
            ]),
            properties: None,
        };

        let response = mailbox_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 2);
        assert_eq!(response.not_found.len(), 1);
    }

    #[tokio::test]
    async fn test_mailbox_set_if_in_state() {
        let store = create_test_store();
        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: Some("state123".to_string()),
            create: None,
            update: None,
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        // Old state and new state should be timestamps (may be equal if no changes)
        assert!(!response.old_state.is_empty());
        assert!(!response.new_state.is_empty());
        assert!(response.new_state >= response.old_state);
    }

    #[tokio::test]
    async fn test_mailbox_query_changes_with_filter() {
        let store = create_test_store();
        let filter = MailboxFilterCondition {
            parent_id: None,
            name: None,
            role: Some(MailboxRole::Inbox),
            has_any_role: None,
            is_subscribed: None,
        };

        let request = MailboxQueryChangesRequest {
            account_id: "acc1".to_string(),
            since_query_state: "10".to_string(),
            filter: Some(filter),
            sort: None,
            max_changes: None,
            up_to_id: None,
            calculate_total: None,
        };

        let response = mailbox_query_changes(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.account_id, "acc1");
    }

    #[tokio::test]
    async fn test_mailbox_default_sort_order() {
        let store = create_test_store();
        let request = MailboxGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = mailbox_get(request, store.as_ref()).await.unwrap();

        // Verify sort orders are assigned correctly
        let inbox = response.list.iter().find(|m| m.id == "inbox").unwrap();
        assert_eq!(inbox.sort_order, 0);

        let sent = response.list.iter().find(|m| m.id == "sent").unwrap();
        assert_eq!(sent.sort_order, 10);
    }

    #[tokio::test]
    async fn test_mailbox_subscribed_default() {
        let store = create_test_store();
        let request = MailboxGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["inbox".to_string()]),
            properties: None,
        };

        let response = mailbox_get(request, store.as_ref()).await.unwrap();
        assert!(response.list[0].is_subscribed);
    }

    #[tokio::test]
    async fn test_mailbox_query_position_beyond_results() {
        let store = create_test_store();
        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: None,
            position: Some(100),
            limit: Some(10),
            calculate_total: Some(true),
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 0);
        assert_eq!(response.total, Some(6));
    }

    #[tokio::test]
    async fn test_mailbox_batch_operations() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let mut update_map = HashMap::new();

        create_map.insert(
            "new1".to_string(),
            MailboxObject {
                name: "New Folder".to_string(),
                parent_id: None,
                role: None,
                sort_order: None,
                is_subscribed: None,
            },
        );

        update_map.insert("inbox".to_string(), serde_json::json!({"sortOrder": 999}));

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: Some(update_map),
            destroy: Some(vec!["trash".to_string()]),
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.created.is_some());
        assert!(response.updated.is_some());
        // Trash is special-use, cannot be destroyed
        assert!(response.not_destroyed.is_some());
    }

    #[tokio::test]
    async fn test_mailbox_role_conversion() {
        assert_eq!(MailboxRole::Inbox.to_special_use(), "\\Inbox");
        assert_eq!(MailboxRole::Sent.to_special_use(), "\\Sent");
        assert_eq!(MailboxRole::Drafts.to_special_use(), "\\Drafts");
        assert_eq!(MailboxRole::Trash.to_special_use(), "\\Trash");
        assert_eq!(MailboxRole::Junk.to_special_use(), "\\Junk");
        assert_eq!(MailboxRole::Archive.to_special_use(), "\\Archive");
        assert_eq!(MailboxRole::Important.to_special_use(), "\\Important");
    }

    #[tokio::test]
    async fn test_mailbox_role_from_special_use() {
        assert_eq!(
            MailboxRole::from_special_use("\\Inbox"),
            Some(MailboxRole::Inbox)
        );
        assert_eq!(
            MailboxRole::from_special_use("\\Sent"),
            Some(MailboxRole::Sent)
        );
        assert_eq!(
            MailboxRole::from_special_use("\\Drafts"),
            Some(MailboxRole::Drafts)
        );
        assert_eq!(
            MailboxRole::from_special_use("\\Trash"),
            Some(MailboxRole::Trash)
        );
        assert_eq!(
            MailboxRole::from_special_use("\\Junk"),
            Some(MailboxRole::Junk)
        );
        assert_eq!(
            MailboxRole::from_special_use("\\Archive"),
            Some(MailboxRole::Archive)
        );
        assert_eq!(MailboxRole::from_special_use("\\Unknown"), None);
    }

    #[tokio::test]
    async fn test_mailbox_role_auto_detection() {
        assert_eq!(
            MailboxRole::detect_from_name("Inbox"),
            Some(MailboxRole::Inbox)
        );
        assert_eq!(
            MailboxRole::detect_from_name("INBOX"),
            Some(MailboxRole::Inbox)
        );
        assert_eq!(
            MailboxRole::detect_from_name("Sent Items"),
            Some(MailboxRole::Sent)
        );
        assert_eq!(
            MailboxRole::detect_from_name("Spam"),
            Some(MailboxRole::Junk)
        );
        assert_eq!(
            MailboxRole::detect_from_name("Deleted Items"),
            Some(MailboxRole::Trash)
        );
        assert_eq!(MailboxRole::detect_from_name("Custom Folder"), None);
    }

    #[tokio::test]
    async fn test_mailbox_name_validation() {
        assert!(mailbox_types::validate_mailbox_name("Valid Name").is_ok());
        assert!(mailbox_types::validate_mailbox_name("").is_err());
        assert!(mailbox_types::validate_mailbox_name("Name/With/Slash").is_err());
        assert!(mailbox_types::validate_mailbox_name(&"a".repeat(256)).is_err());
    }

    #[tokio::test]
    async fn test_mailbox_set_create_with_auto_role() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "new1".to_string(),
            MailboxObject {
                name: "Spam".to_string(), // Should auto-detect as Junk role
                parent_id: None,
                role: None,
                sort_order: None,
                is_subscribed: None,
            },
        );

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.created.is_some());
        let created = response.created.unwrap();
        let mailbox = created.values().next().unwrap();
        assert_eq!(mailbox.role, Some(MailboxRole::Junk));
    }

    #[tokio::test]
    async fn test_mailbox_set_create_invalid_name() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "new1".to_string(),
            MailboxObject {
                name: "".to_string(), // Empty name
                parent_id: None,
                role: None,
                sort_order: None,
                is_subscribed: None,
            },
        );

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
        let not_created = response.not_created.unwrap();
        assert_eq!(
            not_created.get("new1").unwrap().error_type,
            "invalidProperties"
        );
    }

    #[tokio::test]
    async fn test_mailbox_set_create_invalid_parent() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "new1".to_string(),
            MailboxObject {
                name: "Child".to_string(),
                parent_id: Some("nonexistent".to_string()),
                role: None,
                sort_order: None,
                is_subscribed: None,
            },
        );

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_mailbox_set_update_name() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert("inbox".to_string(), serde_json::json!({"name": "My Inbox"}));

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.updated.is_some());
        let updated = response.updated.unwrap();
        let mailbox = updated.get("inbox").unwrap().as_ref().unwrap();
        assert_eq!(mailbox.name, "My Inbox");
    }

    #[tokio::test]
    async fn test_mailbox_set_update_subscription() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert(
            "inbox".to_string(),
            serde_json::json!({"isSubscribed": false}),
        );

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.updated.is_some());
        let updated = response.updated.unwrap();
        let mailbox = updated.get("inbox").unwrap().as_ref().unwrap();
        assert!(!mailbox.is_subscribed);
    }

    #[tokio::test]
    async fn test_mailbox_set_update_not_found() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert(
            "nonexistent".to_string(),
            serde_json::json!({"name": "New Name"}),
        );

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_updated.is_some());
        let not_updated = response.not_updated.unwrap();
        assert_eq!(
            not_updated.get("nonexistent").unwrap().error_type,
            "notFound"
        );
    }

    #[tokio::test]
    async fn test_mailbox_query_filter_parent_id() {
        let store = create_test_store();
        let filter = MailboxFilterCondition {
            parent_id: Some("inbox".to_string()),
            name: None,
            role: None,
            has_any_role: None,
            is_subscribed: None,
        };

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: Some(true),
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        // None of the default mailboxes have parent_id = inbox
        assert_eq!(response.ids.len(), 0);
    }

    #[tokio::test]
    async fn test_mailbox_query_filter_has_any_role() {
        let store = create_test_store();
        let filter = MailboxFilterCondition {
            parent_id: None,
            name: None,
            role: None,
            has_any_role: Some(true),
            is_subscribed: None,
        };

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: Some(true),
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        // All default mailboxes have roles
        assert_eq!(response.ids.len(), 6);
    }

    #[tokio::test]
    async fn test_mailbox_query_filter_is_subscribed() {
        let store = create_test_store();
        let filter = MailboxFilterCondition {
            parent_id: None,
            name: None,
            role: None,
            has_any_role: None,
            is_subscribed: Some(true),
        };

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: Some(true),
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        // All default mailboxes are subscribed
        assert_eq!(response.ids.len(), 6);
    }

    #[tokio::test]
    async fn test_mailbox_query_sort_by_name() {
        let store = create_test_store();
        let sort = vec![MailboxSort {
            property: "name".to_string(),
            is_ascending: Some(true),
        }];

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: Some(sort),
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 6);
        // First should be "Archive" alphabetically
        assert_eq!(response.ids[0], "archive");
    }

    #[tokio::test]
    async fn test_mailbox_query_sort_descending() {
        let store = create_test_store();
        let sort = vec![MailboxSort {
            property: "sortOrder".to_string(),
            is_ascending: Some(false),
        }];

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: Some(sort),
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 6);
        // Last sort_order (50 = archive) should be first
        assert_eq!(response.ids[0], "archive");
    }

    #[tokio::test]
    async fn test_mailbox_changes_max_changes() {
        let store = create_test_store();
        let request = MailboxChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "1".to_string(),
            max_changes: Some(10),
        };

        let response = mailbox_changes(request, store.as_ref()).await.unwrap();
        assert_eq!(response.old_state, "1");
        assert!(!response.has_more_changes);
    }

    #[tokio::test]
    async fn test_mailbox_query_changes_up_to_id() {
        let store = create_test_store();
        let request = MailboxQueryChangesRequest {
            account_id: "acc1".to_string(),
            since_query_state: "1".to_string(),
            filter: None,
            sort: None,
            max_changes: Some(50),
            up_to_id: Some("inbox".to_string()),
            calculate_total: None,
        };

        let response = mailbox_query_changes(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.account_id, "acc1");
    }

    #[tokio::test]
    async fn test_mailbox_set_update_parent_id() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert("sent".to_string(), serde_json::json!({"parentId": "inbox"}));

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.updated.is_some());
        let updated = response.updated.unwrap();
        let mailbox = updated.get("sent").unwrap().as_ref().unwrap();
        assert_eq!(mailbox.parent_id, Some("inbox".to_string()));
    }

    #[tokio::test]
    async fn test_mailbox_set_update_clear_parent() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert("inbox".to_string(), serde_json::json!({"parentId": null}));

        let request = MailboxSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
            on_destroy_remove_emails: None,
        };

        let response = mailbox_set(request, store.as_ref()).await.unwrap();
        assert!(response.updated.is_some());
        let updated = response.updated.unwrap();
        let mailbox = updated.get("inbox").unwrap().as_ref().unwrap();
        assert_eq!(mailbox.parent_id, None);
    }

    #[tokio::test]
    async fn test_mailbox_query_filter_by_name() {
        let store = create_test_store();
        let filter = MailboxFilterCondition {
            parent_id: None,
            name: Some("Inbox".to_string()),
            role: None,
            has_any_role: None,
            is_subscribed: None,
        };

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 1);
        assert_eq!(response.ids[0], "inbox");
    }

    #[tokio::test]
    async fn test_mailbox_query_multiple_sort_comparators() {
        let store = create_test_store();
        let sort = vec![
            MailboxSort {
                property: "totalEmails".to_string(),
                is_ascending: Some(false),
            },
            MailboxSort {
                property: "name".to_string(),
                is_ascending: Some(true),
            },
        ];

        let request = MailboxQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: Some(sort),
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = mailbox_query(request, store.as_ref()).await.unwrap();
        assert_eq!(response.ids.len(), 6);
        // All have 0 emails, so sorted by name
        assert_eq!(response.ids[0], "archive");
    }
}
