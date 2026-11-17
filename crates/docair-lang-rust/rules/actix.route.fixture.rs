// Fixture for actix.route.yaml. Every `//>` line is a fact the rule must
// find, and it must find no others.

use actix_web::{get, post, web, HttpResponse};

#[post("/api/orders")]
pub async fn create_order(order: web::Json<Order>) -> HttpResponse {
    HttpResponse::Ok().finish()
}

/// Other attributes may stand around the route.
#[get("/api/orders/{id}")]
#[tracing::instrument]
async fn get_order(id: web::Path<u32>) -> HttpResponse {
    HttpResponse::Ok().finish()
}

#[delete("/api/orders/{id}", wrap = "Auth")]
async fn cancel_order(id: web::Path<u32>) -> HttpResponse {
    HttpResponse::Ok().finish()
}

pub struct Admin;

impl Admin {
    #[put("/admin/limits")]
    pub async fn set_limits(&self) {}
}

// Not routes: another attribute, a macro that only looks alike, a name that is no method.
#[cfg(test)]
fn helper() {}

#[derive(Debug)]
struct Order;

#[route("/x", method = "GET")]
async fn generic_route() {}

//> serves http POST /api/orders by fn create_order
//> serves http GET /api/orders/{id} by fn get_order
//> serves http DELETE /api/orders/{id} by fn cancel_order
//> serves http PUT /admin/limits by fn set_limits
