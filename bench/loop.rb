def main
  t = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  acc = 0
  n = 0
  while n < 10_000_000
    acc += n * 2 - 1
    n += 1
  end
  puts "RESULT #{acc} #{Process.clock_gettime(Process::CLOCK_MONOTONIC) - t}"
end

main
