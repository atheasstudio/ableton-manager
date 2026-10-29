"""Ableton Manager : surface de contrôle qui laisse l'appli piloter Live.

À copier dans « Documents/Ableton/User Library/Remote Scripts/AbletonManager »,
puis à choisir dans Préférences > Link, Tempo & MIDI > Control Surface.
L'appli sait l'installer elle-même (onglet Timeline d'un projet).

Inspiré d'AbletonOSC (https://github.com/ideoforms/AbletonOSC), en plus court.
"""

import Live
from ableton.v2.control_surface import ControlSurface

from .handlers import Handlers
from .server import Server


def _logger(c_instance):
    """Écrit dans Log.txt ; log_message n'existe pas sur le ControlSurface v2."""

    def log(message):
        try:
            c_instance.log_message(message)
        except Exception:
            pass

    return log


def create_instance(c_instance):
    return AbletonManager(c_instance)


class AbletonManager(ControlSurface):
    def __init__(self, c_instance):
        ControlSurface.__init__(self, c_instance)
        handlers = Handlers(lambda: self.song, Live.Application.get_application)
        self._server = Server(handlers, log=_logger(c_instance))
        self._server.poll()
        self.show_message("Ableton Manager connecté")

    def update_display(self):
        ControlSurface.update_display(self)
        self._server.poll()

    def disconnect(self):
        self._server.close()
        ControlSurface.disconnect(self)
