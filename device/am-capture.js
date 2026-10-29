// Ableton Manager Capture : enregistre la sortie du master pendant la
// lecture et envoie à l'appli la position dans le morceau, pour assembler
// les aperçus. Sans l'appli, rien n'est enregistré.
//
// Protocole (OSC en local) : voir src-tauri/src/capture.rs.

autowatch = 0;
inlets = 1;
// 0 : messages vers l'appli (udpsend), 1 : sfrecord~, 2 : texte d'état
outlets = 3;

var song = null;
// Dossier des captures, donné par l'appli.
var folder = null;
var lastReply = 0;
// Écoute en cours : { n, started, path, render, aborted }
var capture = null;
var counter = 0;
// Rendu à la demande en cours : { end, loop }
var render = null;

var poller = new Task(poll, this);
poller.interval = 50;
var hello = new Task(sayHello, this);
hello.interval = 5000;

// live.thisdevice : le device est prêt, l'API de Live est utilisable.
function bang() {
    if (song) return;
    song = new LiveAPI(onSong, "live_set");
    song.property = "is_playing";
    sayHello();
    hello.repeat();
    status();
}

function get(prop) {
    var v = song.get(prop);
    return v instanceof Array ? Number(v[0]) : Number(v);
}

// Appelle `fn` hors d'une notification de Live (qui interdit de modifier le set).
function later(fn) {
    var t = new Task(fn, this);
    t.schedule(10);
}

function sayHello() {
    // L'appli ne répond plus : on arrête d'enregistrer.
    if (folder && Date.now() - lastReply > 15000) {
        folder = null;
        status();
    }
    outlet(0, "/am/hello");
}

function onSong(args) {
    if (!song || args[0] !== "is_playing") return;
    if (Number(args[1]) === 1) start();
    else stop();
}

function start() {
    if (capture || !folder) {
        status();
        return;
    }
    counter++;
    var path = folder + "/cap-" + Date.now() + "-" + counter + ".wav";
    capture = { n: counter, started: Date.now(), path: path, render: !!render, aborted: false };
    outlet(1, "open", path, "wave");
    outlet(1, 1);
    var end = render ? render.end : get("last_event_time");
    outlet(0, "/am/start", capture.n, path, get("tempo"), end, capture.render ? 1 : 0);
    poll();
    poller.repeat();
    status();
}

function poll() {
    if (!capture) return;
    var beats = get("current_song_time");
    outlet(0, "/am/pos", capture.n, Date.now() - capture.started, beats);
    if (capture.render && render && beats >= render.end) {
        song.call("stop_playing");
    }
}

function stop() {
    if (!capture) return;
    poller.cancel();
    outlet(1, 0);
    var c = capture;
    capture = null;
    if (c.render) endRender();
    // Laisse sfrecord~ fermer le fichier avant de prévenir l'appli.
    later(function () {
        outlet(0, c.aborted ? "/am/abort" : "/am/stop", c.n);
    });
    status();
}

function startRender() {
    if (!song || !folder) return;
    var end = get("last_event_time");
    if (!(end > 0)) {
        outlet(0, "/am/error", "L'arrangement est vide : rien à rendre.");
        return;
    }
    if (capture) song.call("stop_playing");
    later(function () {
        // Queue de 4 temps pour les réverbs et les delays.
        render = { end: end + 4, loop: get("loop") };
        song.set("loop", 0);
        song.set("back_to_arranger", 0);
        song.call("stop_playing");
        song.set("current_song_time", 0);
        song.call("start_playing");
        status();
    });
}

function cancelRender() {
    if (capture && capture.render) {
        capture.aborted = true;
        song.call("stop_playing");
    }
}

function endRender() {
    var r = render;
    render = null;
    if (r) {
        later(function () {
            song.set("loop", r.loop);
        });
    }
}

function anything() {
    var a = arrayfromargs(arguments);
    if (messagename === "/am/folder") {
        folder = String(a[0]);
        lastReply = Date.now();
        status();
    } else if (messagename === "/am/render") {
        startRender();
    } else if (messagename === "/am/cancel") {
        cancelRender();
    }
}

function status() {
    var text;
    if (!folder) text = "Appli fermée";
    else if (capture && capture.render) text = "Rendu en cours…";
    else if (capture) text = "● Écoute enregistrée";
    else text = "Prêt";
    outlet(2, "set", text);
}

// Device retiré ou set fermé : on ferme le fichier.
function notifydeleted() {
    poller.cancel();
    hello.cancel();
    if (capture) outlet(1, 0);
}
