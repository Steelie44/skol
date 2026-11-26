-- A mixed "game frame" workload: AI state machines, vector physics, a
-- spatial-hash collision pass, sorting, event callbacks, UI string building
-- and parsing, string-keyed inventory counts, and BFS pathfinding.
local N, FRAMES, WORLD, CELL, GRID = 500, 1000, 512.0, 32, 16
local sqrt, min = math.sqrt, math.min

local seed = 12345
local function rnd(n)
  seed = (seed * 75 + 74) % 65537
  return seed % n
end

local Entity = {}
Entity.__index = Entity

function Entity.new(i)
  local e = setmetatable({}, Entity)
  e.id = i
  e.x = rnd(512) + 0.0
  e.y = rnd(512) + 0.0
  e.vx = rnd(5) - 2.0
  e.vy = rnd(5) - 2.0
  e.hp = 50 + rnd(50)
  e.state = 0
  return e
end

function Entity:think(px, py)
  local dx, dy = px - self.x, py - self.y
  local d2 = dx * dx + dy * dy
  if d2 >= 10000.0 then self.state = 0
  elseif self.hp < 30 then self.state = 2
  else self.state = 1 end
  if self.state ~= 0 and d2 > 0.0 then
    local d = sqrt(d2)
    if self.state == 1 then
      self.vx = dx / d * 2.0
      self.vy = dy / d * 2.0
    else
      self.vx = -dx / d * 3.0
      self.vy = -dy / d * 3.0
    end
  end
end

function Entity:integrate()
  self.x = self.x + self.vx
  self.y = self.y + self.vy
  if self.x < 0.0 then self.x = 0.0; self.vx = -self.vx
  elseif self.x > WORLD - 1.0 then self.x = WORLD - 1.0; self.vx = -self.vx end
  if self.y < 0.0 then self.y = 0.0; self.vy = -self.vy
  elseif self.y > WORLD - 1.0 then self.y = WORLD - 1.0; self.vy = -self.vy end
end

local function bfs(walls, size)
  local dist = {}
  for k = 0, size * size - 1 do dist[k] = -1 end
  local queue = { [0] = 0 }
  local qlen = 1
  dist[0] = 0
  local head = 0
  local goal = size * size - 1
  local dxs, dys = { 1, -1, 0, 0 }, { 0, 0, 1, -1 }
  while head < qlen do
    local cur = queue[head]
    head = head + 1
    if cur == goal then return dist[cur] end
    local cx, cy = cur % size, cur // size
    for k = 1, 4 do
      local nx, ny = cx + dxs[k], cy + dys[k]
      if nx >= 0 and nx < size and ny >= 0 and ny < size then
        local nxt = ny * size + nx
        if walls[nxt] == 0 and dist[nxt] < 0 then
          dist[nxt] = dist[cur] + 1
          queue[qlen] = nxt
          qlen = qlen + 1
        end
      end
    end
  end
  return -1
end

local score, combo, best_combo = 0, 0, 0

local function main()
  local entities = {}
  for i = 0, N - 1 do entities[i] = Entity.new(i) end
  local walls = {}
  for k = 0, 32 * 32 - 1 do walls[k] = (rnd(100) < 25) and 1 or 0 end
  walls[0] = 0
  walls[32 * 32 - 1] = 0
  local listeners = {
    function(id) score = score + 10 + id % 3 end,
    function(id) combo = combo + 1; if combo > best_combo then best_combo = combo end end,
    function(id) if id % 50 == 0 then score = score + 100 end end,
  }
  local inventory = {}
  local grid = {}
  for k = 0, GRID * GRID - 1 do grid[k] = {} end
  local hits, kills, checksum = 0, 0, 0

  local t = os.clock()
  for frame = 0, FRAMES - 1 do
    local px = 64.0 + (frame * 37) % 384
    local py = 64.0 + (frame * 23) % 384

    for i = 0, N - 1 do
      local e = entities[i]
      e:think(px, py)
      e:integrate()
    end

    local counts = {}
    for k = 0, GRID * GRID - 1 do counts[k] = 0 end
    for i = 0, N - 1 do
      local e = entities[i]
      local cx = min(math.floor(e.x) // CELL, GRID - 1)
      local cy = min(math.floor(e.y) // CELL, GRID - 1)
      local c = cy * GRID + cx
      local n = counts[c] + 1
      counts[c] = n
      grid[c][n] = e.id
    end

    combo = 0
    for c = 0, GRID * GRID - 1 do
      local cell, n = grid[c], counts[c]
      for a = 1, n do
        local ea = entities[cell[a]]
        for b = a + 1, n do
          local eb = entities[cell[b]]
          local dx, dy = ea.x - eb.x, ea.y - eb.y
          if dx * dx + dy * dy < 64.0 then
            hits = hits + 1
            -- push the pair apart
            ea.x = ea.x + dx * 0.5; ea.y = ea.y + dy * 0.5
            eb.x = eb.x - dx * 0.5; eb.y = eb.y - dy * 0.5
            local pair = { ea, eb }
            for p = 1, 2 do
              local e = pair[p]
              e.hp = e.hp - 1
              if e.hp <= 0 then
                e.hp = 100
                kills = kills + 1
                for l = 1, #listeners do listeners[l](e.id) end
              end
            end
          end
        end
      end
    end

    if frame % 10 == 0 then
      local keyed = {}
      for i = 0, N - 1 do
        local e = entities[i]
        local dx, dy = e.x - px, e.y - py
        keyed[i + 1] = { math.floor(dx * dx + dy * dy) * 1024 + e.id, e }
      end
      table.sort(keyed, function(p, q) return p[1] < q[1] end)
      for k = 1, 10 do checksum = checksum + keyed[k][2].id end
    end

    if frame % 5 == 0 then
      local total_hp = 0
      for i = 0, N - 1 do total_hp = total_hp + entities[i].hp end
      local line = "frame " .. frame .. " hp " .. total_hp .. " kills " .. kills
      checksum = checksum + #line
      local parts = {}
      for word in string.gmatch(line, "[^ ]+") do parts[#parts + 1] = word end
      checksum = checksum + tonumber(parts[2]) + tonumber(parts[4]) % 1000 + tonumber(parts[6])
    end

    local item = "item" .. rnd(20)
    inventory[item] = (inventory[item] or 0) + 1

    if frame % 25 == 0 then checksum = checksum + bfs(walls, 32) end
  end

  checksum = checksum + hits + kills * 7 + score + best_combo + (inventory["item7"] or 0)
  print(string.format("RESULT %d %.6f", checksum, os.clock() - t))
end

main()
