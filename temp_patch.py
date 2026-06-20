def main():
    with open('src/components/ArchiverDashboard.tsx', 'r') as f:
        content = f.read()

    old = '  // without updating the branches above breaks the build.\n  const _exhaustive: never = status;\n  return <div className="h-4 w-4 shrink-0 rounded-full bg-gray-700" />;'
    new = '  // without updating the branches above breaks the build.\n  const _exhaustive: never = status;\n  void _exhaustive; // suppress "declared but never read\u201d; proves exhaustive check is real\n  return <div className="h-4 w-4 shrink-0 rounded-full bg-gray-700" />;'

    if old in content:
        content = content.replace(old, new, 1)
        with open('src/components/ArchiverDashboard.tsx', 'w') as f:
            f.write(content)
        print('Success')
    else:
        print('Not found')

if __name__ == '__main__':
    main()
