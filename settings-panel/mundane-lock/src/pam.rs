//! The only producer of an unlock packet is the successful system PAM transaction.
use super::{Event, Question};
use async_channel::Sender;
use pam_client::{Context, ConversationHandler, ErrorCode, Flag};
use std::{
    ffi::{CStr, CString},
    os::unix::net::UnixDatagram,
    time::Duration,
};

struct Conversation {
    events: Sender<Event>,
}
impl Conversation {
    fn ask(&mut self, label: &CStr, echo: bool) -> Result<CString, ErrorCode> {
        let (reply, rx) = async_channel::bounded(1);
        self.events
            .send_blocking(Event::Question(Question {
                label: label.to_string_lossy().into_owned(),
                echo,
                reply,
            }))
            .map_err(|_| ErrorCode::CONV_ERR)?;
        let answer = rx.recv_blocking().map_err(|_| ErrorCode::CONV_ERR)?;
        CString::new(answer.as_bytes()).map_err(|_| ErrorCode::CONV_ERR)
    }
}
impl ConversationHandler for Conversation {
    fn prompt_echo_on(&mut self, label: &CStr) -> Result<CString, ErrorCode> {
        self.ask(label, true)
    }
    fn prompt_echo_off(&mut self, label: &CStr) -> Result<CString, ErrorCode> {
        self.ask(label, false)
    }
    fn text_info(&mut self, text: &CStr) {
        let _ = self
            .events
            .send_blocking(Event::Status(text.to_string_lossy().into_owned()));
    }
    fn error_msg(&mut self, text: &CStr) {
        self.text_info(text);
    }
}
pub fn run(user: String, epoch: u64, channel: UnixDatagram, events: Sender<Event>) {
    loop {
        let conv = Conversation {
            events: events.clone(),
        };
        let success = Context::new("mundane-lock", Some(&user), conv)
            .and_then(|mut pam| {
                pam.authenticate(Flag::DISALLOW_NULL_AUTHTOK)?;
                pam.acct_mgmt(Flag::NONE)?;
                pam.user()
            })
            .is_ok_and(|account| account == user);
        if success {
            // Stay alive until the compositor retires this connection. It owns the mask.
            if channel
                .send(format!("authenticated/1 {epoch}").as_bytes())
                .is_err()
            {
                let _ = events.send_blocking(Event::Lost);
            }
            return;
        }
        if events
            .send_blocking(Event::Status("Unlock failed. Try again.".into()))
            .is_err()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(550));
    }
}
