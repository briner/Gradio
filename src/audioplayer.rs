extern crate gstreamer;
extern crate gtk;
extern crate glib;
use glib::prelude::*;
use gstreamer::{Element, ElementFactory, ElementExt, Bus, Message, Continue, MessageView};
use gstreamer::prelude::*;
use rustio::station::Station;
use std::thread;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::mpsc::{channel, Sender, Receiver};
use rustio::client::Client;

use app_cache::AppCache;
use app_state::AppState;

use mdl::Model;

pub struct AudioPlayer{
    app_cache: AppCache,
    playbin: Element,
    client: Client,
    stream: Rc<RefCell<String>>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum PlaybackState{
    Playing,
    Stopped,
    Loading,

    // We need a own state to start / to stop the playback, otherwise we would create a signal/callback loop
    SetPlaying,
    SetStopped,
}

impl AudioPlayer{
    pub fn new(app_cache: AppCache) -> AudioPlayer{
        gstreamer::init();

        let playbin = ElementFactory::make("playbin", "playbin").unwrap();
        let bus = playbin.get_bus().expect("Unable to get playbin bus");
        let client = Client::new();
        let stream = Rc::new(RefCell::new("".to_string()));

        Self::new_bus_messages(app_cache.clone(), bus);

        let player = AudioPlayer{
            app_cache,
            playbin,
            client,
            stream,
        };

        player.connect_signals();
        player
    }

    fn connect_signals(&self){
        // set initial playback state, we use gtk timeout here, otherwise arc/mutex is locked by sth else
        let app_cache = self.app_cache.clone();
        gtk::timeout_add(1, move ||{
            let c = &*app_cache.get_cache();
            let mut app_state = AppState::get(c, "app").unwrap();
            app_state.ap_state = PlaybackState::Stopped;
            app_state.store(c);
            app_cache.emit_signal("ap-playback".to_string());
            gtk::Continue(false)
        });

        // Playback //
        let app_cache = self.app_cache.clone();
        let playbin = self.playbin.clone();
        self.app_cache.signaler.subscribe("ap-playback", Box::new(move |sig| {
            let c = &*app_cache.get_cache();
            let app_state = AppState::get(c, "app").unwrap();

            match app_state.ap_state{
                PlaybackState::SetPlaying => { playbin.set_state(gstreamer::State::Playing); },
                PlaybackState::SetStopped => { playbin.set_state(gstreamer::State::Ready); },
                _ => (),
            };
        }));

        // Station //
        let app_cache = self.app_cache.clone();
        let stream = self.stream.clone();
        let playbin = self.playbin.clone();
        self.app_cache.signaler.subscribe("ap-station", Box::new(move |sig| {
            let c = &*app_cache.get_cache();
            let mut app_state = AppState::get(c, "app").unwrap();

            let new_station = app_state.ap_station.clone().unwrap();

           debug!("set station for playback: {:?}", new_station);
           *stream.borrow_mut() = new_station.clone().url;

           app_state.ap_title = None;
           app_state.store(c);
           app_cache.emit_signal("ap-title".to_string());

           playbin.set_state(gstreamer::State::Null);
           let p = playbin.clone();
           thread::spawn(move||{
               let station_url = Client::get_playable_station_url(&new_station);
               p.set_property("uri", &station_url);
               p.set_state(gstreamer::State::Playing);
           });
        }));
    }

    fn parse_message(message: &Message, app_cache: AppCache){
        match message.view(){
            MessageView::Tag(tag) => {
                tag.get_tags().get::<gstreamer::tags::Title>().map(|title| {
                    debug!("playback title changed: {:?}", title);
                    let c = &*app_cache.get_cache();
                    AppState::get(c, "app").map(|mut a|{
                        a.ap_title = Some(title.get().unwrap().to_string());
                        a.store(c);
                    });
                    app_cache.emit_signal("ap-title".to_string());
                });
            },
            MessageView::StateChanged(sc) => {
                debug!("playback state changed: {:?}", sc.get_current());
                let c = &*app_cache.get_cache();
                let mut app_state = AppState::get(c, "app").unwrap();

                match sc.get_current(){
                    gstreamer::State::Playing => app_state.ap_state = PlaybackState::Playing,
                    gstreamer::State::Paused => app_state.ap_state = PlaybackState::Loading,
                    _ => app_state.ap_state = PlaybackState::Stopped,
                };

                app_state.store(c);
                app_cache.emit_signal("ap-playback".to_string());
            }
            _ => (),
        };
    }

    fn new_bus_messages (app_cache: AppCache, bus: gstreamer::Bus){
        gtk::timeout_add(250, move ||{
            while(bus.have_pending()){
                bus.pop().map(|message| Self::parse_message(&message, app_cache.clone()) );
            }
            Continue(true)
        });
    }
}
