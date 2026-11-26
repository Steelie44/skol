# A mixed "game frame" workload: AI state machines, vector physics, a
# spatial-hash collision pass, sorting, event callbacks, UI string building
# and parsing, string-keyed inventory counts, and BFS pathfinding.
N = 500
FRAMES = 1000
WORLD = 512.0
CELL = 32
GRID = 16

$seed = 12345
def rnd(n)
  $seed = ($seed * 75 + 74) % 65537
  $seed % n
end

class Entity
  attr_accessor :id, :x, :y, :vx, :vy, :hp, :state

  def initialize(i)
    @id = i
    @x = rnd(512).to_f
    @y = rnd(512).to_f
    @vx = rnd(5).to_f - 2.0
    @vy = rnd(5).to_f - 2.0
    @hp = 50 + rnd(50)
    @state = 0
  end

  def think(px, py)
    dx = px - @x
    dy = py - @y
    d2 = dx * dx + dy * dy
    if d2 >= 10000.0
      @state = 0
    elsif @hp < 30
      @state = 2
    else
      @state = 1
    end
    if @state != 0 && d2 > 0.0
      d = Math.sqrt(d2)
      if @state == 1
        @vx = dx / d * 2.0
        @vy = dy / d * 2.0
      else
        @vx = -dx / d * 3.0
        @vy = -dy / d * 3.0
      end
    end
  end

  def integrate
    @x += @vx
    @y += @vy
    if @x < 0.0
      @x = 0.0
      @vx = -@vx
    elsif @x > WORLD - 1.0
      @x = WORLD - 1.0
      @vx = -@vx
    end
    if @y < 0.0
      @y = 0.0
      @vy = -@vy
    elsif @y > WORLD - 1.0
      @y = WORLD - 1.0
      @vy = -@vy
    end
  end
end

def bfs(walls, size)
  dist = Array.new(size * size, -1)
  queue = [0]
  dist[0] = 0
  head = 0
  goal = size * size - 1
  dxs = [1, -1, 0, 0]
  dys = [0, 0, 1, -1]
  while head < queue.length
    cur = queue[head]
    head += 1
    return dist[cur] if cur == goal
    cx = cur % size
    cy = cur / size
    4.times do |k|
      nx = cx + dxs[k]
      ny = cy + dys[k]
      if nx >= 0 && nx < size && ny >= 0 && ny < size
        nxt = ny * size + nx
        if walls[nxt] == 0 && dist[nxt] < 0
          dist[nxt] = dist[cur] + 1
          queue << nxt
        end
      end
    end
  end
  -1
end

$score = 0
$combo = 0
$best_combo = 0

def main
  entities = Array.new(N) { |i| Entity.new(i) }
  walls = Array.new(32 * 32) { rnd(100) < 25 ? 1 : 0 }
  walls[0] = 0
  walls[32 * 32 - 1] = 0
  listeners = [
    ->(id) { $score += 10 + id % 3 },
    ->(id) { $combo += 1; $best_combo = $combo if $combo > $best_combo },
    ->(id) { $score += 100 if id % 50 == 0 },
  ]
  inventory = {}
  grid = Array.new(GRID * GRID) { [] }
  hits = 0
  kills = 0
  checksum = 0

  t = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  FRAMES.times do |frame|
    px = (64 + (frame * 37) % 384).to_f
    py = (64 + (frame * 23) % 384).to_f

    entities.each do |e|
      e.think(px, py)
      e.integrate
    end

    grid.each(&:clear)
    entities.each do |e|
      cx = [e.x.to_i / CELL, GRID - 1].min
      cy = [e.y.to_i / CELL, GRID - 1].min
      grid[cy * GRID + cx] << e.id
    end

    $combo = 0
    grid.each do |cell|
      n = cell.length
      a = 0
      while a < n
        ea = entities[cell[a]]
        b = a + 1
        while b < n
          eb = entities[cell[b]]
          dx = ea.x - eb.x
          dy = ea.y - eb.y
          if dx * dx + dy * dy < 64.0
            hits += 1
            # push the pair apart
            ea.x += dx * 0.5
            ea.y += dy * 0.5
            eb.x -= dx * 0.5
            eb.y -= dy * 0.5
            [ea, eb].each do |e|
              e.hp -= 1
              if e.hp <= 0
                e.hp = 100
                kills += 1
                listeners.each { |cb| cb.call(e.id) }
              end
            end
          end
          b += 1
        end
        a += 1
      end
    end

    if frame % 10 == 0
      near = entities.sort_by do |e|
        dx = e.x - px
        dy = e.y - py
        (dx * dx + dy * dy).to_i * 1024 + e.id
      end
      near.first(10).each { |e| checksum += e.id }
    end

    if frame % 5 == 0
      total_hp = 0
      entities.each { |e| total_hp += e.hp }
      line = "frame " + frame.to_s + " hp " + total_hp.to_s + " kills " + kills.to_s
      checksum += line.length
      parts = line.split(" ")
      checksum += parts[1].to_i + parts[3].to_i % 1000 + parts[5].to_i
    end

    item = "item" + rnd(20).to_s
    inventory[item] = (inventory[item] || 0) + 1

    checksum += bfs(walls, 32) if frame % 25 == 0
  end

  checksum += hits + kills * 7 + $score + $best_combo + (inventory["item7"] || 0)
  puts "RESULT #{checksum} #{Process.clock_gettime(Process::CLOCK_MONOTONIC) - t}"
end

main
