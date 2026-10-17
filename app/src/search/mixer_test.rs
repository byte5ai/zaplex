use super::*;
use crate::auth::AuthManager;
use crate::auth::AuthStateProvider;
use crate::search::item::SearchItem;
use ordered_float::OrderedFloat;
use std::time::Duration;
use warpui::r#async::Timer;
use warpui::{App, AppContext, Element};

#[derive(Clone, Debug, PartialEq)]
struct TestAction {
    id: String,
}

#[derive(Debug)]
struct TestSearchItem {
    id: String,
    priority_tier: u8,
    score: f64,
    dedup_key: Option<String>,
}

impl SearchItem for TestSearchItem {
    type Action = TestAction;

    fn render_icon(
        &self,
        _highlight_state: crate::search::result_renderer::ItemHighlightState,
        _appearance: &crate::appearance::Appearance,
    ) -> Box<dyn Element> {
        unimplemented!()
    }

    fn render_item(
        &self,
        _highlight_state: crate::search::result_renderer::ItemHighlightState,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        unimplemented!()
    }

    fn priority_tier(&self) -> u8 {
        self.priority_tier
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.score)
    }

    fn accept_result(&self) -> Self::Action {
        TestAction {
            id: self.id.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        TestAction {
            id: self.id.clone(),
        }
    }

    fn accessibility_label(&self) -> String {
        self.id.clone()
    }

    fn dedup_key(&self) -> Option<String> {
        self.dedup_key.clone()
    }
}

struct StaticSyncSource {
    result: TestSearchItem,
}

impl SyncDataSource for StaticSyncSource {
    type Action = TestAction;

    fn run_query(
        &self,
        _: &Query,
        _: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        Ok(vec![QueryResult::from(TestSearchItem {
            id: self.result.id.clone(),
            priority_tier: self.result.priority_tier,
            score: self.result.score,
            dedup_key: self.result.dedup_key.clone(),
        })])
    }
}

struct DelayedAsyncSource {
    delay: Duration,
    result: TestSearchItem,
}

impl AsyncDataSource for DelayedAsyncSource {
    type Action = TestAction;

    fn run_query(
        &self,
        _: &Query,
        _: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        let delay = self.delay;
        let id = self.result.id.clone();
        let priority_tier = self.result.priority_tier;
        let score = self.result.score;
        let dedup_key = self.result.dedup_key.clone();
        Box::pin(async move {
            Timer::after(delay).await;
            Ok(vec![QueryResult::from(TestSearchItem {
                id,
                priority_tier,
                score,
                dedup_key,
            })])
        })
    }
}

struct QueryDrivenDelayedAsyncSource;

impl AsyncDataSource for QueryDrivenDelayedAsyncSource {
    type Action = TestAction;

    fn run_query(
        &self,
        query: &Query,
        _: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        let (delay, id) = match query.text.as_str() {
            "first" => (Duration::from_millis(200), "stale_first".to_string()),
            "second" => (Duration::from_millis(300), "fresh_second".to_string()),
            text => (Duration::from_millis(50), text.to_string()),
        };
        Box::pin(async move {
            Timer::after(delay).await;
            Ok(vec![QueryResult::from(TestSearchItem {
                id,
                priority_tier: 0,
                score: 0.0,
                dedup_key: None,
            })])
        })
    }
}

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

#[test]
fn test_dedupe_on_keeps_highest_score() {
    // Add items with same dedup key but different scores
    let results = vec![
        QueryResult::from(TestSearchItem {
            id: "item1".to_string(),
            priority_tier: 0,
            score: 1.0,
            dedup_key: Some("key1".to_string()),
        }),
        QueryResult::from(TestSearchItem {
            id: "item2".to_string(),
            priority_tier: 0,
            score: 3.0,
            dedup_key: Some("key1".to_string()),
        }),
        QueryResult::from(TestSearchItem {
            id: "item3".to_string(),
            priority_tier: 0,
            score: 2.0,
            dedup_key: Some("key1".to_string()),
        }),
    ];

    let results = dedupe_score(results);
    // Should keep only the item with highest score (item2 with score 3.0)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].accept_result().id, "item2");
}

#[test]
fn test_dedupe_on_preserves_items_without_keys() {
    // Add items with and without dedup keys
    let results = vec![
        QueryResult::from(TestSearchItem {
            id: "item1".to_string(),
            priority_tier: 0,
            score: 1.0,
            dedup_key: None,
        }),
        QueryResult::from(TestSearchItem {
            id: "item2".to_string(),
            priority_tier: 0,
            score: 2.0,
            dedup_key: None,
        }),
        QueryResult::from(TestSearchItem {
            id: "item3".to_string(),
            priority_tier: 0,
            score: 3.0,
            dedup_key: Some("key1".to_string()),
        }),
    ];

    let results = dedupe_score(results);
    // Should keep all items without dedup keys
    assert_eq!(results.len(), 3);
}
