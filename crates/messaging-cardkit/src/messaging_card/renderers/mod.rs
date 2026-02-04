pub mod platform;

pub use platform::PlatformRenderer;

#[derive(Debug, Clone, Copy, Default)]
pub struct SlackRenderer;

#[derive(Debug, Clone, Copy, Default)]
pub struct TeamsRenderer;

#[derive(Debug, Clone, Copy, Default)]
pub struct TelegramRenderer;

#[derive(Debug, Clone, Copy, Default)]
pub struct WhatsAppRenderer;

#[derive(Debug, Clone, Copy, Default)]
pub struct WebChatRenderer;

#[derive(Debug, Clone, Copy, Default)]
pub struct WebexRenderer;
