"""Record Orca's actual speech requests in the disposable test user's runtime."""

import json
import os
from pathlib import Path
from time import monotonic
from orca.speechserver import SpeechServer as BaseServer


class SpeechServer(BaseServer):
    @classmethod
    def get_speech_server(cls, info=None):
        return cls("mundane-test")

    def speak(self, text=None, acss=None):
        if text:
            with (Path(os.environ["XDG_RUNTIME_DIR"]) / "mundane-speech.jsonl").open(
                "a"
            ) as stream:
                print(json.dumps([monotonic(), text]), file=stream)

    def speak_character(self, character, acss=None, cap_style=None):
        self.speak(character)

    def speak_key_event(self, event, acss=None):
        self.speak(event.get_key_name())
