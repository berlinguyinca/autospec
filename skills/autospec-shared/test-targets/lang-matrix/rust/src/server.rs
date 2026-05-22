// server.rs — Actix-web HTTP server
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use crate::lib::greet;

#[get("/greet/{name}")]
async fn greet_handler(name: web::Path<String>) -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({ "message": greet(&name) }))
}

pub async fn start_server(addr: &str) -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(greet_handler))
        .bind(addr)?
        .run()
        .await
}
