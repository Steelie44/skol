<?php
// A mixed "game frame" workload: AI state machines, vector physics, a
// spatial-hash collision pass, sorting, event callbacks, UI string building
// and parsing, string-keyed inventory counts, and BFS pathfinding.
const N = 500;
const FRAMES = 1000;
const WORLD = 512.0;
const CELL = 32;
const GRID = 16;

$seed = 12345;
function rnd(int $n): int {
    global $seed;
    $seed = ($seed * 75 + 74) % 65537;
    return $seed % $n;
}

class Entity {
    public int $id;
    public float $x;
    public float $y;
    public float $vx;
    public float $vy;
    public int $hp;
    public int $state = 0;

    public function __construct(int $i) {
        $this->id = $i;
        $this->x = (float) rnd(512);
        $this->y = (float) rnd(512);
        $this->vx = (float) rnd(5) - 2.0;
        $this->vy = (float) rnd(5) - 2.0;
        $this->hp = 50 + rnd(50);
    }

    public function think(float $px, float $py): void {
        $dx = $px - $this->x;
        $dy = $py - $this->y;
        $d2 = $dx * $dx + $dy * $dy;
        if ($d2 >= 10000.0) $this->state = 0;
        elseif ($this->hp < 30) $this->state = 2;
        else $this->state = 1;
        if ($this->state != 0 && $d2 > 0.0) {
            $d = sqrt($d2);
            if ($this->state == 1) { $this->vx = $dx / $d * 2.0; $this->vy = $dy / $d * 2.0; }
            else { $this->vx = -$dx / $d * 3.0; $this->vy = -$dy / $d * 3.0; }
        }
    }

    public function integrate(): void {
        $this->x += $this->vx;
        $this->y += $this->vy;
        if ($this->x < 0.0) { $this->x = 0.0; $this->vx = -$this->vx; }
        elseif ($this->x > WORLD - 1.0) { $this->x = WORLD - 1.0; $this->vx = -$this->vx; }
        if ($this->y < 0.0) { $this->y = 0.0; $this->vy = -$this->vy; }
        elseif ($this->y > WORLD - 1.0) { $this->y = WORLD - 1.0; $this->vy = -$this->vy; }
    }
}

function bfs(array $walls, int $size): int {
    $dist = array_fill(0, $size * $size, -1);
    $queue = [0];
    $dist[0] = 0;
    $head = 0;
    $goal = $size * $size - 1;
    $dxs = [1, -1, 0, 0];
    $dys = [0, 0, 1, -1];
    while ($head < count($queue)) {
        $cur = $queue[$head++];
        if ($cur == $goal) return $dist[$cur];
        $cx = $cur % $size;
        $cy = intdiv($cur, $size);
        for ($k = 0; $k < 4; $k++) {
            $nx = $cx + $dxs[$k];
            $ny = $cy + $dys[$k];
            if ($nx >= 0 && $nx < $size && $ny >= 0 && $ny < $size) {
                $nxt = $ny * $size + $nx;
                if ($walls[$nxt] == 0 && $dist[$nxt] < 0) {
                    $dist[$nxt] = $dist[$cur] + 1;
                    $queue[] = $nxt;
                }
            }
        }
    }
    return -1;
}

$score = 0;
$combo = 0;
$bestCombo = 0;

function main(): void {
    global $score, $combo, $bestCombo;
    $entities = [];
    for ($i = 0; $i < N; $i++) $entities[] = new Entity($i);
    $walls = [];
    for ($k = 0; $k < 32 * 32; $k++) $walls[] = rnd(100) < 25 ? 1 : 0;
    $walls[0] = 0;
    $walls[32 * 32 - 1] = 0;
    $listeners = [
        function (int $id) use (&$score) { $score += 10 + $id % 3; },
        function (int $id) use (&$combo, &$bestCombo) { $combo += 1; if ($combo > $bestCombo) $bestCombo = $combo; },
        function (int $id) use (&$score) { if ($id % 50 == 0) $score += 100; },
    ];
    $inventory = [];
    $grid = array_fill(0, GRID * GRID, []);
    $hits = 0;
    $kills = 0;
    $checksum = 0;

    $t = hrtime(true);
    for ($frame = 0; $frame < FRAMES; $frame++) {
        $px = (float) (64 + ($frame * 37) % 384);
        $py = (float) (64 + ($frame * 23) % 384);

        foreach ($entities as $e) { $e->think($px, $py); $e->integrate(); }

        for ($k = 0; $k < GRID * GRID; $k++) $grid[$k] = [];
        foreach ($entities as $e) {
            $cx = min(intdiv((int) $e->x, CELL), GRID - 1);
            $cy = min(intdiv((int) $e->y, CELL), GRID - 1);
            $grid[$cy * GRID + $cx][] = $e->id;
        }

        $combo = 0;
        foreach ($grid as $cell) {
            $n = count($cell);
            for ($a = 0; $a < $n; $a++) {
                $ea = $entities[$cell[$a]];
                for ($b = $a + 1; $b < $n; $b++) {
                    $eb = $entities[$cell[$b]];
                    $dx = $ea->x - $eb->x;
                    $dy = $ea->y - $eb->y;
                    if ($dx * $dx + $dy * $dy < 64.0) {
                        $hits += 1;
                        // push the pair apart
                        $ea->x += $dx * 0.5; $ea->y += $dy * 0.5;
                        $eb->x -= $dx * 0.5; $eb->y -= $dy * 0.5;
                        foreach ([$ea, $eb] as $e) {
                            $e->hp -= 1;
                            if ($e->hp <= 0) {
                                $e->hp = 100;
                                $kills += 1;
                                foreach ($listeners as $cb) $cb($e->id);
                            }
                        }
                    }
                }
            }
        }

        if ($frame % 10 == 0) {
            $keyed = [];
            foreach ($entities as $e) {
                $dx = $e->x - $px;
                $dy = $e->y - $py;
                $keyed[] = [(int) ($dx * $dx + $dy * $dy) * 1024 + $e->id, $e];
            }
            usort($keyed, fn($p, $q) => $p[0] <=> $q[0]);
            for ($k = 0; $k < 10; $k++) $checksum += $keyed[$k][1]->id;
        }

        if ($frame % 5 == 0) {
            $totalHp = 0;
            foreach ($entities as $e) $totalHp += $e->hp;
            $line = "frame " . $frame . " hp " . $totalHp . " kills " . $kills;
            $checksum += strlen($line);
            $parts = explode(" ", $line);
            $checksum += (int) $parts[1] + (int) $parts[3] % 1000 + (int) $parts[5];
        }

        $item = "item" . rnd(20);
        if (isset($inventory[$item])) $inventory[$item] += 1; else $inventory[$item] = 1;

        if ($frame % 25 == 0) $checksum += bfs($walls, 32);
    }

    $checksum += $hits + $kills * 7 + $score + $bestCombo + ($inventory["item7"] ?? 0);
    printf("RESULT %d %.6f\n", $checksum, (hrtime(true) - $t) / 1e9);
}

main();
