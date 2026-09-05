//! Starting a session from the terminal a person is at.
//!
//! The request is `corral_client::launch`'s, the same one every surface
//! makes. What this surface adds is where it asks from: a session born at
//! this terminal's size and in this process's directory, both facts only a
//! terminal surface can read for itself.

use corral_client::launch::{LaunchSite, Requested};
use corral_client::{Connection, RequestError};
use corral_protocol::method::SessionNewResult;

use crate::attach::Geometry;

/// Ask `corrald` to start a session here.
pub async fn start_session(
    connection: &mut Connection,
    requested: Requested,
) -> Result<SessionNewResult, RequestError> {
    let geometry = Geometry::of(std::io::stdin());
    corral_client::launch::start_session(
        connection,
        requested,
        LaunchSite {
            working_directory: std::env::current_dir().ok(),
            rows: geometry.map(|geometry| geometry.rows),
            cols: geometry.map(|geometry| geometry.cols),
        },
    )
    .await
}
