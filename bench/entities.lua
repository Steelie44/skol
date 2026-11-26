local Enemy = {}
Enemy.__index = Enemy

function Enemy.new(i)
  return setmetatable({
    x = (i % 100) + 0.0,
    y = math.floor(i / 100) + 0.0,
    vx = 1.0 + (i % 7) * 0.1,
    vy = 0.5 + (i % 5) * 0.1,
    hp = 100,
  }, Enemy)
end

function Enemy:integrate(dt)
  self.x = self.x + self.vx * dt
  self.y = self.y + self.vy * dt
  if self.x > 100.0 or self.x < 0.0 then self.vx = -self.vx end
  if self.y > 100.0 or self.y < 0.0 then self.vy = -self.vy end
end

function Enemy:damage(amount)
  self.hp = self.hp - amount
  if self.hp <= 0 then self.hp = 100 end
end

local sqrt = math.sqrt

local function update(enemies, px, py, dt)
  local hits = 0
  for i = 1, #enemies do
    local e = enemies[i]
    e:integrate(dt)
    local dx = e.x - px
    local dy = e.y - py
    if sqrt(dx * dx + dy * dy) < 10.0 then
      e:damage(1)
      hits = hits + 1
    end
  end
  return hits
end

local enemies = {}
for k = 0, 999 do enemies[#enemies + 1] = Enemy.new(k) end
local total = 0
local t = os.clock()
for frame = 0, 999 do
  total = total + update(enemies, 50.0 + (frame % 20), 50.0, 0.5)
end
print(string.format("RESULT %d %.6f", total, os.clock() - t))
