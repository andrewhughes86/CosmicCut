"""A block with a quarter-twisted top (original), in millimetres, Z up, standing on z = 0.

Each side is a twisted face made of level straight lines: the shape a swarf cut follows, the
table turning as the wire goes down.
Makes samples/twisted-block.stl: python3 tools/twisted_block.py samples/twisted-block.stl
"""
import math, struct, sys

WIDTH, DEPTH, HEIGHT, TWIST, ROWS = 160.0, 100.0, 180.0, 45.0, 120

def corner(k, z):
    x, y = [(-WIDTH / 2, -DEPTH / 2), (WIDTH / 2, -DEPTH / 2), (WIDTH / 2, DEPTH / 2), (-WIDTH / 2, DEPTH / 2)][k % 4]
    a = math.radians(TWIST * z / HEIGHT)
    return (x * math.cos(a) - y * math.sin(a), x * math.sin(a) + y * math.cos(a), z)

tris = []
for r in range(ROWS):
    z0, z1 = HEIGHT * r / ROWS, HEIGHT * (r + 1) / ROWS
    for k in range(4):
        a, b, c, d = corner(k, z0), corner(k + 1, z0), corner(k + 1, z1), corner(k, z1)
        tris += [(a, b, c), (a, c, d)]
bottom = [corner(k, 0.0) for k in range(4)]
top = [corner(k, HEIGHT) for k in range(4)]
tris += [(bottom[0], bottom[2], bottom[1]), (bottom[0], bottom[3], bottom[2])]
tris += [(top[0], top[1], top[2]), (top[0], top[2], top[3])]

def normal(a, b, c):
    u = [b[i] - a[i] for i in range(3)]
    v = [c[i] - a[i] for i in range(3)]
    n = (u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0])
    l = math.sqrt(sum(x * x for x in n)) or 1.0
    return tuple(x / l for x in n)

with open(sys.argv[1], "wb") as f:
    f.write(b"CosmicCut twisted block".ljust(80, b" "))
    f.write(struct.pack("<I", len(tris)))
    for t in tris:
        f.write(struct.pack("<3f", *normal(*t)))
        for p in t:
            f.write(struct.pack("<3f", *p))
        f.write(b"\0\0")
