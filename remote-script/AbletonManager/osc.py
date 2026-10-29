"""Codage et décodage des messages OSC (sous-ensemble utile : i, f, d, s, T, F, N).

Aucune dépendance à Live : ce module se teste avec un Python ordinaire.
"""

import struct


def _padded(raw):
    """Chaîne OSC : terminée par un octet nul, puis complétée à 4 octets."""
    raw += b"\0"
    return raw + b"\0" * (-len(raw) % 4)


def _read_string(data, i):
    end = data.index(b"\0", i)
    text = data[i:end].decode("utf-8")
    end += 1
    return text, end + (-end % 4)


def encode(address, args=()):
    tags = ","
    payload = b""
    for arg in args:
        # bool avant int : True est aussi un int en Python.
        if arg is True:
            tags += "T"
        elif arg is False:
            tags += "F"
        elif arg is None:
            tags += "N"
        elif isinstance(arg, int):
            tags += "i"
            payload += struct.pack(">i", arg)
        elif isinstance(arg, float):
            tags += "f"
            payload += struct.pack(">f", arg)
        else:
            tags += "s"
            payload += _padded(str(arg).encode("utf-8"))
    return _padded(address.encode("utf-8")) + _padded(tags.encode("ascii")) + payload


def decode(data):
    """Renvoie (adresse, arguments). Lève ValueError si le message est invalide."""
    try:
        address, i = _read_string(data, 0)
        if not address.startswith("/"):
            raise ValueError("adresse OSC invalide")
        if i >= len(data):
            return address, []
        tags, i = _read_string(data, i)
        args = []
        for tag in tags[1:]:
            if tag == "i":
                args.append(struct.unpack_from(">i", data, i)[0])
                i += 4
            elif tag == "f":
                args.append(struct.unpack_from(">f", data, i)[0])
                i += 4
            elif tag == "d":
                args.append(struct.unpack_from(">d", data, i)[0])
                i += 8
            elif tag == "s":
                text, i = _read_string(data, i)
                args.append(text)
            elif tag == "T":
                args.append(True)
            elif tag == "F":
                args.append(False)
            elif tag == "N":
                args.append(None)
            else:
                raise ValueError("type OSC non géré : " + tag)
        return address, args
    except (struct.error, UnicodeDecodeError) as e:
        raise ValueError(str(e))
