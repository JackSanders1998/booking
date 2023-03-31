use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use booking::{Pagination, UpdateVenueItem, VenueItem, VenueStore, VenueStoreError};
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Type for our shared state
///
/// In our sample application, we store the venue list in memory. As the state is shared
/// between concurrently running web requests, we need to make it thread-safe.
type Db = Arc<RwLock<VenueStore>>;

#[tokio::main]
async fn main() {
    // Enable tracing using Tokio's https://tokio.rs/#tk-lib-tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "venue_axum=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Create shared data store
    let db = Db::default();

    // We register our shared state so that handlers can get it using the State extractor.
    // Note that this will change in Axum 0.6. See more at
    // https://docs.rs/axum/0.6.0-rc.4/axum/index.html#sharing-state-with-handlers
    let app = Router::new()
        // Here we setup the routes. Note: No macros
        .route("/venues", get(get_venues).post(add_venue))
        .route("/venues/:id", delete(delete_venue).patch(update_venue).get(get_venue))
        .with_state(db)
        // Using tower to add tracing layer
        .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()).into_inner());

    // In practice: Use graceful shutdown.
    // Note that Axum has great examples for a log of practical scenarios,
    // including graceful shutdown (https://github.com/tokio-rs/axum/tree/main/examples)
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr).serve(app.into_make_service()).await.unwrap();
}

/// Get list of venue items
///
/// Note how the Query extractor is used to get query parameters. Note how the State
/// extractor is used to get the database (changes in Axum 0.6 RC).
/// Extractors are technically types that implement FromRequest. You can create
/// your own extractors or use the ones provided by Axum.
async fn get_venues(pagination: Option<Query<Pagination>>, State(db): State<Db>) -> impl IntoResponse {
    let venues = db.read().await;
    let Query(pagination) = pagination.unwrap_or_default();
    // Json is an extractor and a response.
    Json(venues.get_venues(pagination))
}

/// Get a single venue item
///
/// Note how the Path extractor is used to get query parameters.
async fn get_venue(Path(id): Path<usize>, State(db): State<Db>) -> impl IntoResponse {
    let venues = db.read().await;
    if let Some(item) = venues.get_venue(id) {
        // Note how to return Json
        Json(item).into_response()
    } else {
        // Note how a tuple can be turned into a response
        (StatusCode::NOT_FOUND, "Not found").into_response()
    }
}

/// Add a new venue item
///
/// Note that this time, Json is used as an extractor. This means that the request body
/// will be deserialized into a VenueItem.
async fn add_venue(State(db): State<Db>, Json(venue): Json<VenueItem>) -> impl IntoResponse {
    let mut venues = db.write().await;
    let venue = venues.add_venue(venue);
    (StatusCode::CREATED, Json(venue))
}

/// Delete a venue
async fn delete_venue(Path(id): Path<usize>, State(db): State<Db>) -> impl IntoResponse {
    if db.write().await.remove_venue(id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Update a venue
async fn update_venue(
    Path(id): Path<usize>,
    State(db): State<Db>,
    Json(input): Json<UpdateVenueItem>,
) -> Result<impl IntoResponse, StatusCode> {
    let mut venues = db.write().await;
    let res = venues.update_venue(&id, input);
    match res {
        Some(venue) => Ok(Json(venue.clone())),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// Application-level error object
enum AppError {
    UserRepo(VenueStoreError),
}
impl From<VenueStoreError> for AppError {
    fn from(inner: VenueStoreError) -> Self {
        AppError::UserRepo(inner)
    }
}

/// Logic for turning an error into a response.
///
/// By providing this trait, handlers can return AppError and Axum will automatically
/// convert it into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AppError::UserRepo(VenueStoreError::FileAccessError(_)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Error while writing to file")
            },
            AppError::UserRepo(VenueStoreError::SerializationError(_)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Error during serialization")
            },
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}