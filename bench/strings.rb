def main
  counts = Hash.new(0)
  total = 0
  t = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  200_000.times do |k|
    key = "k" + (k % 1000).to_s
    total += key.length
    counts[key] += 1
  end
  puts "RESULT #{total + counts.size} #{Process.clock_gettime(Process::CLOCK_MONOTONIC) - t}"
end

main
