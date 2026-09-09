use tracepress_core::{MaxIpcFrameBytes, MaxRequestBodyBytes, MaxResponseBodyBytes};

/// Validated framing and opaque-body limits for one IPC connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcLimits {
    frame: MaxIpcFrameBytes,
    request_body: MaxRequestBodyBytes,
    response_body: MaxResponseBodyBytes,
}

impl IpcLimits {
    /// Groups the independent core limits enforced at the IPC boundary.
    #[must_use]
    pub const fn new(
        maximum_frame: MaxIpcFrameBytes,
        maximum_request_body: MaxRequestBodyBytes,
        maximum_response_body: MaxResponseBodyBytes,
    ) -> Self {
        Self {
            frame: maximum_frame,
            request_body: maximum_request_body,
            response_body: maximum_response_body,
        }
    }

    /// Returns the maximum serialized frame size.
    #[must_use]
    pub const fn maximum_frame(self) -> MaxIpcFrameBytes {
        self.frame
    }

    /// Returns the maximum opaque request body size.
    #[must_use]
    pub const fn maximum_request_body(self) -> MaxRequestBodyBytes {
        self.request_body
    }

    /// Returns the maximum opaque response body size.
    #[must_use]
    pub const fn maximum_response_body(self) -> MaxResponseBodyBytes {
        self.response_body
    }
}
