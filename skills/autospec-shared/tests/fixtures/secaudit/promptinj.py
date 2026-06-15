import openai

def ask(user_msg):
    prompt = "Summarize this for the user: " + user_msg
    return openai.ChatCompletion.create(model="gpt-4", messages=[{"role": "user", "content": prompt}])
