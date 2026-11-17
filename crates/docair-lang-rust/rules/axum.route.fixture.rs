// Fixture for axum.route.yaml. Every `//>` line is a fact the rule must
// find, and it must find no others.

use axum::Router;
use axum::routing::{get, get_service, post};

mod handlers {
    pub async fn cancel_order() {}
}

pub fn router(files: ServeDir) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/api/orders", get(list_orders).post(create_order))
        .route("/api/orders/{id}", get(get_order).put(update_order).delete(handlers::cancel_order))
        // Not a method router: no facts.
        .route("/static", get_service(files))
        // A closure has no name to link: no facts.
        .route("/version", get(|| async { "1" }))
        .layer(Trace)
}

async fn health() -> &'static str {
    "ok"
}
async fn list_orders() {}
async fn create_order() {}
async fn get_order() {}
async fn update_order() {}

//> serves http GET /healthz by fn health
//> serves http GET /api/orders by fn list_orders
//> serves http POST /api/orders by fn create_order
//> serves http GET /api/orders/{id} by fn get_order
//> serves http PUT /api/orders/{id} by fn update_order
//> serves http DELETE /api/orders/{id} by fn cancel_order
//> calls fn health from fn router (GET /healthz)
//> calls fn list_orders from fn router (GET /api/orders)
//> calls fn create_order from fn router (POST /api/orders)
//> calls fn get_order from fn router (GET /api/orders/{id})
//> calls fn update_order from fn router (PUT /api/orders/{id})
//> calls fn cancel_order from fn router (DELETE /api/orders/{id})
