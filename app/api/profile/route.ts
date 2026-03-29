import { NextResponse } from 'next/server';
import { prisma } from '@/lib/prisma';

export async function PUT(req: Request) {
  try {
    const userAddress = req.headers.get('x-wallet-address'); 
    if (!userAddress) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

    const body = await req.json();
    const { username, email } = body;

    if (!username || !email) {
      return NextResponse.json({ error: 'Invalid body' }, { status: 400 });
    }

    const user = await prisma.user.upsert({
      where: { address: userAddress },
      update: { username, email },
      create: { address: userAddress, username, email },
    });

    return NextResponse.json(user, { status: 200 });
  } catch (error) {
    console.error('Database mutation failed:', error);
    return NextResponse.json({ error: 'Database mutation failed' }, { status: 500 });
  }
}

export async function GET(req: Request) {
  try {
    const userAddress = req.headers.get('x-wallet-address');
    if (!userAddress) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

    const user = await prisma.user.findUnique({
      where: { address: userAddress },
      select: {
        id: true,
        email: true,
        username: true,
        address: true,
        firstName: true,
        lastName: true,
      }
    });

    if (!user) return NextResponse.json({ error: 'User not found' }, { status: 404 });

    return NextResponse.json(user, { status: 200 });
  } catch (error) {
    return NextResponse.json({ error: 'Internal server error' }, { status: 500 });
  }
}
