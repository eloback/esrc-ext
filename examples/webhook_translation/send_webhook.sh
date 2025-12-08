curl -X POST http://localhost:3000/webhook/create_user \
     -H "Content-Type: application/json" \
     -d '{
           "id": "550e8400-e29b-41d4-a716-446655440000",
           "name": "John Doe",
           "email": "john.doe@example.com"
         }'