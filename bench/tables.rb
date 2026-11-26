def main
  grid = Array.new(1024, 0)
  names = {}
  64.times { |k| names["e#{k}"] = k }
  t = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  acc = 0
  2000.times do |rnd|
    k = 0
    while k < 1024
      grid[k] = grid[k] + k
      k += 1
    end
    acc += names["e" + (rnd % 64).to_s]
  end
  puts "RESULT #{acc + grid[1023]} #{Process.clock_gettime(Process::CLOCK_MONOTONIC) - t}"
end

main
