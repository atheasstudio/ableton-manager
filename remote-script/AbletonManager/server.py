"""Serveur OSC/UDP local, interrogé à chaque rafraîchissement de Live.

Live n'autorise les appels à son API que depuis son fil principal : on ne
lance pas de fil, on lit la socket (non bloquante) depuis update_display().
"""

import errno
import socket

from . import osc

HOST = "127.0.0.1"
PORT = 11040


class Server(object):
    def __init__(self, handlers, log=None, port=PORT):
        self._handlers = handlers
        self._log = log or (lambda message: None)
        self._port = port
        self._socket = None

    def _open(self):
        # L'instance précédente (set rechargé) peut tenir encore le port : on
        # réessaiera au prochain passage.
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            s.setblocking(False)
            s.bind((HOST, self._port))
            self._socket = s
            self._log("Ableton Manager : à l'écoute sur %s:%d" % (HOST, self._port))
        except socket.error as e:
            s.close()
            self._log("Ableton Manager : port %d indisponible (%s)" % (self._port, e))

    def poll(self):
        """Traite tous les messages en attente."""
        if self._socket is None:
            self._open()
            if self._socket is None:
                return
        while True:
            try:
                data, sender = self._socket.recvfrom(65536)
            except socket.error as e:
                # Sous Windows, une réponse vers un client parti renvoie
                # WSAECONNRESET (10054) à la lecture suivante : on l'ignore.
                if e.errno in (errno.EAGAIN, errno.EWOULDBLOCK, 10035):
                    return
                if e.errno in (errno.ECONNRESET, 10054):
                    continue
                self._log("Ableton Manager : erreur de lecture (%s)" % e)
                return
            try:
                address, args = osc.decode(data)
            except ValueError as e:
                self._log("Ableton Manager : message invalide (%s)" % e)
                continue
            reply = self._handlers.handle(address, args)
            if reply is not None:
                try:
                    self._socket.sendto(osc.encode(*reply), sender)
                except socket.error:
                    pass

    def close(self):
        if self._socket is not None:
            self._socket.close()
            self._socket = None
