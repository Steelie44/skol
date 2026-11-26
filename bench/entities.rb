class Enemy
  attr_reader :x, :y

  def initialize(i)
    @x = (i % 100).to_f
    @y = (i / 100).to_f
    @vx = 1.0 + (i % 7) * 0.1
    @vy = 0.5 + (i % 5) * 0.1
    @hp = 100
  end

  def integrate(dt)
    @x += @vx * dt
    @y += @vy * dt
    @vx = -@vx if @x > 100.0 || @x < 0.0
    @vy = -@vy if @y > 100.0 || @y < 0.0
  end

  def damage(amount)
    @hp -= amount
    @hp = 100 if @hp <= 0
  end
end

def update(enemies, px, py, dt)
  hits = 0
  enemies.each do |e|
    e.integrate(dt)
    dx = e.x - px
    dy = e.y - py
    if Math.sqrt(dx * dx + dy * dy) < 10.0
      e.damage(1)
      hits += 1
    end
  end
  hits
end

enemies = (0...1000).map { |k| Enemy.new(k) }
total = 0
t = Process.clock_gettime(Process::CLOCK_MONOTONIC)
1000.times do |frame|
  total += update(enemies, 50.0 + (frame % 20).to_f, 50.0, 0.5)
end
puts "RESULT #{total} #{Process.clock_gettime(Process::CLOCK_MONOTONIC) - t}"
