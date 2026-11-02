"""A chubby low-poly penguin (original), in millimetres, Z up, facing -Y, standing on z = 0."""
# Makes samples/penguin.stl: python3 tools/penguin.py samples/penguin.stl
import math, struct, sys

tris = []

def add(a, b, c):
    tris.append((a, b, c))

def lathe(profile, seg, twist=0.5):
    rings = []
    for r, z in profile:
        rings.append([(r * math.cos(2 * math.pi * (k + twist) / seg), r * math.sin(2 * math.pi * (k + twist) / seg), z)
                      for k in range(seg)])
    out = []
    for i in range(len(rings) - 1):
        lo, hi = rings[i], rings[i + 1]
        for k in range(seg):
            k2 = (k + 1) % seg
            if profile[i][0] > 0:
                out.append((lo[k], lo[k2], hi[k2]))
            if profile[i + 1][0] > 0:
                out.append((lo[k], hi[k2], hi[k]))
    return out

def transform(ts, f):
    return [tuple(f(p) for p in t) for t in ts]

def rx(d):
    a = math.radians(d); c, s = math.cos(a), math.sin(a)
    return lambda p: (p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c)

def ry(d):
    a = math.radians(d); c, s = math.cos(a), math.sin(a)
    return lambda p: (p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c)

def rz(d):
    a = math.radians(d); c, s = math.cos(a), math.sin(a)
    return lambda p: (p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2])

def ellipsoid(c, r, seg=10, lat=6, turn=lambda p: p):
    prof = [(math.sin(math.pi * i / lat), -math.cos(math.pi * i / lat)) for i in range(lat + 1)]
    ts = lathe(prof, seg)
    return transform(ts, lambda p: tuple(a + b for a, b in zip(turn((p[0] * r[0], p[1] * r[1], p[2] * r[2])), c)))

# Body and head in one pear: widest low down, a soft neck, a round head.
body = lathe([(0, 16), (62, 19), (98, 40), (112, 82), (110, 128), (96, 172), (78, 206), (72, 228),
              (74, 252), (70, 280), (56, 306), (32, 322), (0, 328)], 14)
tris += transform(body, lambda p: (p[0], p[1] * 0.86, p[2]))
# A wide, flat beak: upper and lower halves.
tris += ellipsoid((0, -64, 262), (38, 28, 12), seg=10, lat=4)
tris += ellipsoid((0, -60, 250), (31, 23, 8), seg=10, lat=4)
# Eyes, just standing proud of the face.
for x in (-23, 23):
    tris += ellipsoid((x, -55, 294), (15, 8, 21), seg=8, lat=4)
# Short flippers hanging at the sides, tips out and a little forward.
for side in (-1, 1):
    tris += ellipsoid((side * 106, -8, 112), (16, 34, 60), seg=8, lat=5,
                      turn=lambda p, s=side: rz(s * -10)(ry(s * -24)(p)))
# Big flat feet, splayed out.
for side in (-1, 1):
    tris += ellipsoid((side * 50, -50, 13), (44, 64, 13), seg=10, lat=4, turn=lambda p, s=side: rz(s * 26)(p))
# A stubby tail at the back.
tris += ellipsoid((0, 84, 34), (32, 22, 12), seg=8, lat=4, turn=rx(-25))

zmin = min(p[2] for t in tris for p in t)
tris = [tuple((x, y, z - zmin) for (x, y, z) in t) for t in tris]

def normal(a, b, c):
    u = [b[i] - a[i] for i in range(3)]; v = [c[i] - a[i] for i in range(3)]
    n = (u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0])
    l = math.sqrt(sum(x * x for x in n))
    return tuple(x / l for x in n) if l > 1e-9 else None

tris = [t for t in tris if normal(*t)]
with open(sys.argv[1], "wb") as f:
    f.write(b"CosmicCut low-poly penguin (original, MIT OR Apache-2.0)".ljust(80, b" "))
    f.write(struct.pack("<I", len(tris)))
    for t in tris:
        f.write(struct.pack("<3f", *normal(*t)))
        for p in t:
            f.write(struct.pack("<3f", *p))
        f.write(b"\0\0")
xs = [p[0] for t in tris for p in t]; ys = [p[1] for t in tris for p in t]; zs = [p[2] for t in tris for p in t]
print(len(tris), "triangles; size", round(max(xs) - min(xs)), round(max(ys) - min(ys)), round(max(zs) - min(zs)))
