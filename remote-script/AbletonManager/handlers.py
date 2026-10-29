"""Réponses aux messages de l'appli.

Les adresses reprennent celles d'AbletonOSC (/live/song/...) ; celles propres
à Ableton Manager commencent par /manager/. Ce module ne dépend pas de Live :
il reçoit le Song (ou un faux, dans les tests) par une fonction.
"""

# À changer quand les messages changent : l'appli propose alors de réinstaller le script.
VERSION = 2


class Handlers(object):
    def __init__(self, get_song, get_application=None):
        self._song = get_song
        self._application = get_application
        self._routes = {
            "/live/test": self._test,
            "/live/application/get/version": self._app_version,
            "/live/song/get/current_song_time": lambda song, args: [song.current_song_time],
            "/live/song/set/current_song_time": self._set_time,
            "/live/song/get/is_playing": lambda song, args: [bool(song.is_playing)],
            "/live/song/get/tempo": lambda song, args: [song.tempo],
            "/live/song/get/song_length": lambda song, args: [song.song_length],
            "/live/song/get/signature_numerator": lambda song, args: [song.signature_numerator],
            "/live/song/get/signature_denominator": lambda song, args: [song.signature_denominator],
            "/live/song/get/cue_points": self._cue_points,
            "/live/song/start_playing": lambda song, args: song.start_playing(),
            "/live/song/continue_playing": lambda song, args: song.continue_playing(),
            "/live/song/stop_playing": lambda song, args: song.stop_playing(),
            "/manager/version": lambda song, args: [VERSION],
            "/manager/status": self._status,
            "/manager/jump": self._jump,
            "/manager/load_capture": self._load_capture,
        }

    def handle(self, address, args):
        """Renvoie la réponse (adresse, arguments), ou None s'il n'y en a pas."""
        route = self._routes.get(address)
        if route is None:
            return "/live/error", ["adresse inconnue : " + address]
        try:
            result = route(self._song(), args)
        except Exception as e:  # une erreur ne doit jamais arrêter le script
            return "/live/error", ["%s : %s" % (address, e)]
        if result is None:
            return None
        return address, list(result)

    def _test(self, song, args):
        return ["ok"]

    def _app_version(self, song, args):
        app = self._application() if self._application else None
        if app is None:
            return []
        return [app.get_major_version(), app.get_minor_version(), app.get_bugfix_version()]

    def _set_time(self, song, args):
        song.current_song_time = max(0.0, float(args[0]))

    def _cue_points(self, song, args):
        flat = []
        for cue in song.cue_points:
            flat += [cue.name, cue.time]
        return flat

    def _status(self, song, args):
        return [bool(song.is_playing), song.current_song_time, song.tempo]

    def _jump(self, song, args):
        """Place la tête de lecture (en beats) ; lance la lecture si demandé."""
        song.current_song_time = max(0.0, float(args[0]))
        if len(args) > 1 and args[1] and not song.is_playing:
            song.continue_playing()
        return [song.current_song_time]

    def _load_capture(self, song, args):
        """Charge « Ableton Manager Capture » de la User Library sur le Master."""
        app = self._application() if self._application else None
        if app is None:
            raise RuntimeError("application Live indisponible")
        for device in song.master_track.devices:
            if device.name.startswith("Ableton Manager Capture"):
                return ["deja"]
        item = _find(app.browser.user_library, "Ableton Manager Capture", 6)
        if item is None:
            raise RuntimeError("device introuvable dans la User Library : installe-le depuis les réglages")
        song.view.selected_track = song.master_track
        app.browser.load_item(item)
        return ["ok"]


def _find(item, name, depth):
    if item.is_loadable and item.name.startswith(name):
        return item
    if depth == 0:
        return None
    for child in item.children:
        found = _find(child, name, depth - 1)
        if found is not None:
            return found
    return None
