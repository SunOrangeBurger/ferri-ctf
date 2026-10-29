def report(values):
    out = []
    for value in values:
        out.append(chr(value ^ 23))
    return ''.join(out)

numbers = [121, 114, 127, 98, 108, 99, 98, 120, 98, 123, 98, 107, 123, 98, 121, 120, 98, 123, 98, 114, 98, 123, 98, 115, 98, 114, 98, 100, 98, 123, 98, 98, 98]
print(report(numbers))
