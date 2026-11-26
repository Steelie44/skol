local function main()
  local counts = {}
  local size = 0
  local total = 0
  local t = os.clock()
  for k = 0, 199999 do
    local key = "k" .. (k % 1000)
    total = total + #key
    local c = counts[key]
    if c then counts[key] = c + 1 else counts[key] = 1; size = size + 1 end
  end
  print(string.format("RESULT %d %.6f", total + size, os.clock() - t))
end
main()
