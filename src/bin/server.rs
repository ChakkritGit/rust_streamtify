use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use streamtify::{Message, PlayerCommand, PlayerState};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast};

// โครงสร้างเพลงใน Playlist
#[derive(Clone)]
struct Song {
    title: String,
    artist: String,
    duration: u64,
}

// โครงสร้างของ "ห้อง"
struct Room {
    state: Arc<Mutex<PlayerState>>,
    tx: broadcast::Sender<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. สร้าง Playlist (Hardcode ไว้)
    let playlist = Arc::new(vec![
        Song {
            title: "Shape of You".into(),
            artist: "Ed Sheeran".into(),
            duration: 233_000,
        },
        Song {
            title: "Blinding Lights".into(),
            artist: "The Weeknd".into(),
            duration: 200_000,
        },
        Song {
            title: "Levitating".into(),
            artist: "Dua Lipa".into(),
            duration: 203_000,
        },
        Song {
            title: "Stay".into(),
            artist: "Justin Bieber".into(),
            duration: 141_000,
        },
        Song {
            title: "Bohemian Rhapsody".into(),
            artist: "Queen".into(),
            duration: 354_000,
        },
    ]);

    // 2. สมุดจดห้อง (Global Room Map)
    let rooms: Arc<Mutex<HashMap<String, Room>>> = Arc::new(Mutex::new(HashMap::new()));

    let listener = TcpListener::bind("0.0.0.0:9000").await?;
    println!("🏢 Multi-Room Music Server running on port 9000");

    loop {
        // รับ Connection ใหม่
        let (mut socket, addr) = listener.accept().await?;
        let rooms_handle = rooms.clone();
        let playlist_handle = playlist.clone();

        tokio::spawn(async move {
            let (reader, mut writer) = socket.split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            // --- ขั้นตอนที่ 1: Login (รับชื่อห้อง) ---
            if reader.read_line(&mut line).await.unwrap() == 0 {
                return;
            }
            let room_name = line.trim().to_string();
            line.clear();

            println!("👤 New User: {} -> Room: '{}'", addr, room_name);

            // --- ขั้นตอนที่ 2: ดึงห้อง หรือ สร้างห้องใหม่ ---
            let (tx, state) = {
                let mut map = rooms_handle.lock().await;

                if !map.contains_key(&room_name) {
                    println!("✨ Creating NEW room: {}", room_name);

                    // State เริ่มต้น (เพลงแรก)
                    let first_song = &playlist_handle[0];
                    let initial_state = PlayerState {
                        song_title: first_song.title.clone(),
                        artist: first_song.artist.clone(),
                        is_playing: false,
                        progress_ms: 0,
                        duration_ms: first_song.duration,
                        current_index: 0,
                        total_songs: playlist_handle.len(),
                    };

                    let state = Arc::new(Mutex::new(initial_state));
                    let (tx_new, _) = broadcast::channel(100);

                    // --- Spawn Ticker (นาฬิกาประจำห้อง) ---
                    let state_ticker = state.clone();
                    let tx_ticker = tx_new.clone();
                    let pl_ticker = playlist_handle.clone();

                    tokio::spawn(async move {
                        let mut interval = tokio::time::interval(Duration::from_millis(1000));
                        loop {
                            interval.tick().await;

                            let mut s = state_ticker.lock().await;
                            if s.is_playing {
                                s.progress_ms += 1000;
                                // จบเพลง -> Next
                                if s.progress_ms >= s.duration_ms {
                                    s.progress_ms = 0;
                                    s.current_index = (s.current_index + 1) % s.total_songs;
                                    let next_song = &pl_ticker[s.current_index];
                                    s.song_title = next_song.title.clone();
                                    s.artist = next_song.artist.clone();
                                    s.duration_ms = next_song.duration;
                                }
                            }

                            // Broadcast Update (ถ้ามีคนฟังอยู่)
                            if tx_ticker.receiver_count() > 0 {
                                let msg = Message::StateUpdate(s.clone());
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    let _ = tx_ticker.send(json);
                                }
                            }
                        }
                    });

                    map.insert(room_name.clone(), Room { state, tx: tx_new });
                }

                let room = map.get(&room_name).unwrap();
                (room.tx.clone(), room.state.clone())
            };

            // --- ขั้นตอนที่ 3: ส่ง State ปัจจุบันให้ Client ทันที ---
            {
                let s = state.lock().await;
                let msg = Message::StateUpdate(s.clone());
                let json = serde_json::to_string(&msg).unwrap();
                writer.write_all(json.as_bytes()).await.unwrap();
                writer.write_all(b"\n").await.unwrap();
            }

            // --- ขั้นตอนที่ 4: Loop รับส่งข้อมูล ---
            let mut rx = tx.subscribe();

            loop {
                tokio::select! {
                    // A. รับ State จาก Server (ห้องเรา) ส่งให้ Client
                    Ok(msg_json) = rx.recv() => {
                        writer.write_all(msg_json.as_bytes()).await.unwrap();
                        writer.write_all(b"\n").await.unwrap();
                    }

                    // B. รับ Command จาก Client
                    Ok(bytes) = reader.read_line(&mut line) => {
                        if bytes == 0 { break; }
                        if let Ok(Message::Command(cmd)) = serde_json::from_str::<Message>(line.trim()) {
                            let mut s = state.lock().await;
                            match cmd {
                                PlayerCommand::Play => s.is_playing = true,
                                PlayerCommand::Pause => s.is_playing = false,
                                PlayerCommand::Restart => s.progress_ms = 0,
                                PlayerCommand::Next => {
                                    s.progress_ms = 0;
                                    s.current_index = (s.current_index + 1) % s.total_songs;
                                    let song = &playlist_handle[s.current_index];
                                    s.song_title = song.title.clone(); s.artist = song.artist.clone(); s.duration_ms = song.duration;
                                },
                                PlayerCommand::Prev => {
                                    s.progress_ms = 0;
                                    s.current_index = (s.current_index + s.total_songs - 1) % s.total_songs;
                                    let song = &playlist_handle[s.current_index];
                                    s.song_title = song.title.clone(); s.artist = song.artist.clone(); s.duration_ms = song.duration;
                                },
                                PlayerCommand::Seek(ms) => s.progress_ms = ms,
                            }
                        }
                        line.clear();
                    }
                }
            }
        });
    }
}
