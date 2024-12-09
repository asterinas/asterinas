
def calculate_average(file_path):
    try:
        with open(file_path, 'r') as file:
            numbers = [float(line.strip()) for line in file if line.strip()]
        if numbers:
            average = sum(numbers) / len(numbers)
            return average
        else:
            return ""
    except FileNotFoundError:
        return "文件未找到。"
    except ValueError:
        return "文件包含无效数据。"

def calculate_averages(file_path):
    column1 = []
    column2 = []
    
    with open(file_path, 'r') as file:
        for line in file:
            # 去除空白字符并按逗号或空格分割
            values = line.strip().replace(',', ' ').split()
            
            # 转换为数字并添加到各自的列表中
            if len(values) == 2:
                column1.append(float(values[0]))
                column2.append(float(values[1]))
    
    avg1 = max(column1) if column1 else 0
    avg2 = max(column2) if column2 else 0

    return avg1, avg2

file_path = 'output.txt'
#average = calculate_average(file_path)
avg1, avg2 = calculate_averages(file_path)
print(f"{avg1} {avg2/5}")