use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use streamtify::{Message, PlayerCommand, PlayerState};
use tokio::sync::{Mutex, broadcast};
use warp::Filter;

// โครงสร้างข้อมูลเพลงใน Playlist
#[derive(Clone)]
struct Song {
    title: String,
    artist: String,
    duration: u64,
}

// โครงสร้างห้อง
struct Room {
    state: Arc<Mutex<PlayerState>>, // สถานะปัจจุบันของห้อง
    tx: broadcast::Sender<String>,  // ช่องทางส่งข้อมูลหาทุกคนในห้อง
}

#[tokio::main]
async fn main() {
    // 1. สร้าง Playlist (ข้อมูลกลาง)
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

    // 2. สมุดจดห้อง (เก็บสถานะของทุกห้อง)
    let rooms: Arc<Mutex<HashMap<String, Room>>> = Arc::new(Mutex::new(HashMap::new()));

    // 3. Setup Filter ของ Warp เพื่อส่งตัวแปรเข้า Handler
    let rooms_filter = warp::any().map(move || rooms.clone());
    let playlist_filter = warp::any().map(move || playlist.clone());

    // 4. สร้าง Route: ws://localhost:9000/ws/:room_name
    let ws_route = warp::path("ws")
        .and(warp::path::param::<String>()) // รับชื่อห้องจาก URL
        .and(warp::ws()) // บอกว่าเป็น WebSocket
        .and(rooms_filter)
        .and(playlist_filter)
        .map(|room_name: String, ws: warp::ws::Ws, rooms, playlist| {
            // Upgrade connection เป็น WebSocket แล้วเรียกฟังก์ชัน handle_connection
            ws.on_upgrade(move |socket| handle_connection(socket, room_name, rooms, playlist))
        });

    println!("🚀 Server Started on port 9000");
    println!("Waiting for connections...");

    warp::serve(ws_route).run(([0, 0, 0, 0], 9000)).await;
}

// ฟังก์ชันจัดการ Client แต่ละคน
async fn handle_connection(
    ws: warp::ws::WebSocket,
    room_name: String,
    rooms: Arc<Mutex<HashMap<String, Room>>>,
    playlist: Arc<Vec<Song>>,
) {
    // แยก Socket เป็นตัวรับ (ws_rx) และตัวส่ง (ws_tx)
    let (mut ws_tx, mut ws_rx) = ws.split();

    // [LOG] แสดงเมื่อมีคนเชื่อมต่อ
    println!("➕ Client connected to room: '{}'", room_name);

    // --- ส่วนจัดการห้อง (Room Logic) ---
    // เข้าไปเช็คว่าห้องมีหรือยัง ถ้ายังไม่มีให้สร้างใหม่
    let (tx, state) = {
        let mut map = rooms.lock().await;

        if !map.contains_key(&room_name) {
            println!("✨ Creating NEW room: '{}'", room_name);

            // State เริ่มต้น (เพลงแรก)
            let s0 = &playlist[0];
            let initial_state = PlayerState {
                song_title: s0.title.clone(),
                artist: s0.artist.clone(),
                is_playing: false,
                progress_ms: 0,
                duration_ms: s0.duration,
                current_index: 0,
                total_songs: playlist.len(),
            };

            let state = Arc::new(Mutex::new(initial_state));
            let (tx_new, _) = broadcast::channel(100);

            // --- Ticker Task (นาฬิกาประจำห้อง) ---
            let state_ticker = state.clone();
            let tx_ticker = tx_new.clone();
            let pl_ticker = playlist.clone();
            let room_name_log = room_name.clone();

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(1000));
                loop {
                    interval.tick().await;
                    let mut s = state_ticker.lock().await;

                    if s.is_playing {
                        s.progress_ms += 1000;

                        // ถ้าเพลงจบ ให้เล่นเพลงถัดไปอัตโนมัติ
                        if s.progress_ms >= s.duration_ms {
                            println!(
                                "🎵 Song finished in room '{}', playing next...",
                                room_name_log
                            );
                            s.progress_ms = 0;
                            s.current_index = (s.current_index + 1) % s.total_songs;
                            let next = &pl_ticker[s.current_index];
                            s.song_title = next.title.clone();
                            s.artist = next.artist.clone();
                            s.duration_ms = next.duration;
                        }
                    }

                    // Broadcast บอกทุกคนในห้อง (ถ้ามีคนฟังอยู่)
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

        let r = map.get(&room_name).unwrap();
        (r.tx.clone(), r.state.clone())
    };

    // ส่ง State ปัจจุบันให้ Client ทันทีที่เชื่อมต่อเสร็จ
    {
        let s = state.lock().await;
        let msg = Message::StateUpdate(s.clone());
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = ws_tx.send(warp::ws::Message::text(json)).await;
        }
    }

    // Subscribe รอรับข่าวสารจากห้อง
    let mut rx = tx.subscribe();

    // --- Loop หลัก (รับ/ส่ง ข้อมูล) ---
    loop {
        tokio::select! {
            // A. ส่งข้อมูลจาก Server (Room) ไปหา Client (React)
            Ok(msg_str) = rx.recv() => {
                // ถ้าส่งไม่ผ่าน (Client หลุด) ให้จบการทำงาน
                if ws_tx.send(warp::ws::Message::text(msg_str)).await.is_err() {
                    break;
                }
            }

            // B. รับคำสั่งจาก Client (React)
            Some(result) = ws_rx.next() => {
                match result {
                    Ok(msg) => {
                        if msg.is_text() {
                            if let Ok(text) = msg.to_str() {
                                // แปลง JSON เป็น Command
                                if let Ok(Message::Command(cmd)) = serde_json::from_str::<Message>(text) {

                                    // [LOG] แสดงคำสั่งที่ได้รับ
                                    println!("📝 Command from room '{}': {:?}", room_name, cmd);

                                    // อัปเดต State
                                    let mut s = state.lock().await;
                                    match cmd {
                                        PlayerCommand::Play => s.is_playing = true,
                                        PlayerCommand::Pause => s.is_playing = false,

                                        PlayerCommand::Next => {
                                            s.progress_ms = 0;
                                            s.current_index = (s.current_index + 1) % s.total_songs;
                                            let next_song = &playlist[s.current_index];
                                            s.song_title = next_song.title.clone();
                                            s.artist = next_song.artist.clone();
                                            s.duration_ms = next_song.duration;
                                        },

                                        PlayerCommand::Prev => {
                                            s.progress_ms = 0;
                                            s.current_index = (s.current_index + s.total_songs - 1) % s.total_songs;
                                            let prev_song = &playlist[s.current_index];
                                            s.song_title = prev_song.title.clone();
                                            s.artist = prev_song.artist.clone();
                                            s.duration_ms = prev_song.duration;
                                        },

                                        PlayerCommand::Restart => s.progress_ms = 0,
                                        _ => {}
                                    }
                                }
                            }
                        } else if msg.is_close() {
                            break;
                        }
                    },
                    Err(_) => break, // Connection error
                }
            }
        }
    }

    // [LOG] แสดงเมื่อ Client หลุด
    println!("❌ Client disconnected from room: '{}'", room_name);
}
