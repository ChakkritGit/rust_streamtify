use serde::{Deserialize, Serialize};

// ข้อมูลสถานะของห้องเพลง
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlayerState {
    pub song_title: String,
    pub artist: String,
    pub is_playing: bool,
    pub progress_ms: u64,
    pub duration_ms: u64,
    pub current_index: usize,
    pub total_songs: usize,
}

// ข้อความที่ใช้สื่อสาร
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Message {
    StateUpdate(PlayerState), // Server -> Client
    Command(PlayerCommand),   // Client -> Server
}

// คำสั่งควบคุม
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum PlayerCommand {
    Play,
    Pause,
    Next,
    Prev,
    Restart,
    Seek(u64),
}
