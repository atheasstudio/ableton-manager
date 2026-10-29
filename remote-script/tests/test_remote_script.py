"""Tests du Remote Script sans Live : python remote-script/tests/test_remote_script.py"""

import os
import socket
import sys
import time
import types
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

# Modules fournis par Live, remplacés par des coquilles vides.
for name in ("Live", "ableton", "ableton.v2", "ableton.v2.control_surface"):
    sys.modules[name] = types.ModuleType(name)
sys.modules["ableton.v2.control_surface"].ControlSurface = object

from AbletonManager import osc  # noqa: E402
from AbletonManager.handlers import Handlers  # noqa: E402
from AbletonManager.server import Server  # noqa: E402


class Cue(object):
    def __init__(self, name, time):
        self.name = name
        self.time = time


class FakeSong(object):
    def __init__(self):
        self.current_song_time = 0.0
        self.is_playing = False
        self.tempo = 120.0
        self.song_length = 256.0
        self.signature_numerator = 4
        self.signature_denominator = 4
        self.cue_points = [Cue("Drop", 64.0)]

    def continue_playing(self):
        self.is_playing = True

    def start_playing(self):
        self.is_playing = True

    def stop_playing(self):
        self.is_playing = False


class OscTest(unittest.TestCase):
    def test_round_trip(self):
        data = osc.encode("/manager/jump", [160.5, True, 3, "é", None, False])
        self.assertEqual(len(data) % 4, 0)
        self.assertEqual(osc.decode(data), ("/manager/jump", [160.5, True, 3, "é", None, False]))

    def test_known_bytes(self):
        # Exemple de la spécification OSC 1.0.
        data = osc.encode("/oscillator/4/frequency", [440.0])
        self.assertEqual(
            data,
            b"/oscillator/4/frequency\0,f\0\0\x43\xdc\x00\x00",
        )

    def test_without_type_tags(self):
        self.assertEqual(osc.decode(b"/live/test\0\0"), ("/live/test", []))

    def test_invalid(self):
        with self.assertRaises(ValueError):
            osc.decode(b"pas de l'osc")
        with self.assertRaises(ValueError):
            osc.decode(osc.encode("/x", [1])[:-2])


class HandlersTest(unittest.TestCase):
    def setUp(self):
        self.song = FakeSong()
        self.h = Handlers(lambda: self.song)

    def test_jump_without_play(self):
        self.assertEqual(self.h.handle("/manager/jump", [160.0, False]), ("/manager/jump", [160.0]))
        self.assertEqual(self.song.current_song_time, 160.0)
        self.assertFalse(self.song.is_playing)

    def test_jump_and_play(self):
        self.h.handle("/manager/jump", [32.0, True])
        self.assertTrue(self.song.is_playing)

    def test_negative_position(self):
        self.h.handle("/live/song/set/current_song_time", [-4.0])
        self.assertEqual(self.song.current_song_time, 0.0)

    def test_status_and_cues(self):
        self.song.current_song_time = 12.0
        self.assertEqual(self.h.handle("/manager/status", []), ("/manager/status", [False, 12.0, 120.0]))
        self.assertEqual(self.h.handle("/live/song/get/cue_points", []), ("/live/song/get/cue_points", ["Drop", 64.0]))

    def test_errors_are_replies(self):
        self.assertEqual(self.h.handle("/nope", [])[0], "/live/error")
        self.assertEqual(self.h.handle("/manager/jump", [])[0], "/live/error")
        self.assertIsNone(self.h.handle("/live/song/stop_playing", []))


class ServerTest(unittest.TestCase):
    def test_udp(self):
        song = FakeSong()
        server = Server(Handlers(lambda: song), port=0)
        server._open()
        port = server._socket.getsockname()[1]
        client = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        client.settimeout(2)
        try:
            client.sendto(b"garbage", ("127.0.0.1", port))
            client.sendto(osc.encode("/manager/jump", [8.0, True]), ("127.0.0.1", port))
            time.sleep(0.05)
            server.poll()
            data, _ = client.recvfrom(1024)
            self.assertEqual(osc.decode(data), ("/manager/jump", [8.0]))
            self.assertTrue(song.is_playing)
        finally:
            client.close()
            server.close()


if __name__ == "__main__":
    unittest.main()
